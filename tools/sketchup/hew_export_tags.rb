# hew_export_tags.rb — losslessly carry SketchUp names + Tag (Layer) membership
# across COLLADA (.dae) export into Hew.
#
# WHY: SketchUp's COLLADA exporter sanitizes every node/material name to
# [A-Za-z0-9_] — spaces and any other character collapse to "_", and a leading
# "_" may be prepended. That is lossy and irreversible:
#
#     "Pretty Ceilings"           -> "Pretty_Ceilings"
#     '8'' 5/8" Drywall'          -> "_8__5_8__Drywall"
#     "[Metal_Aluminum_Anodized]1"-> "__Metal_Aluminum_Anodized_1"
#
# It also drops Tag/Layer assignments entirely. So Hew's object / material / tag
# lists end up with mangled names and no tags.
#
# WHAT IT DOES: before you export, this rewrites every Group, ComponentInstance,
# ComponentDefinition, and Material name to embed a REVERSIBLE payload that
# survives the [A-Za-z0-9_] sanitizer:
#
#     <readable name>__HEWMETA__<lowercase-hex of UTF-8 JSON>
#
# The JSON is {"n":"<real name>","t":[["Folder","Tag"],...]} — "n" is the true
# name, "t" is the (optional) root-first folder path to the entity's Tag.
# Because the payload is plain hex ([0-9a-f]), it passes through the exporter
# untouched; Hew finds the "__HEWMETA__" marker (matched tolerantly as a run of
# underscores around the token), hex-decodes the JSON, and restores the exact
# name + tags. The readable prefix is kept only so the raw .dae stays legible.
#
# USAGE (in SketchUp: Window > Ruby Console):
#   load "/path/to/hew_export_tags.rb"
#   Hew.encode            # rewrite names (one undoable step)
#   # ...now File > Export > 3D Model... as COLLADA (.dae) into Hew...
#   Hew.decode            # OR just press Ctrl/Cmd+Z once to restore names
#
# It is idempotent: encode strips any prior payload first, so running it twice is
# safe. decode removes the payloads explicitly. `encode_tags`/`decode_tags` are
# kept as aliases for older muscle memory.

require "json"

module Hew
  # Payload marker. Sanitizes to a run of underscores around "HEWMETA"; Hew
  # matches it tolerantly.
  META = "__HEWMETA__".freeze
  # Nesting separator for the legacy tag-only scheme (still stripped on re-runs).
  NEST = "__HEWSEP__".freeze
  # Matches this script's marker plus the prior tag-only (`__HEWTAG__`) and the
  # original `@@HEWTAG@@` markers, so re-runs stay idempotent across versions.
  DELIM_RE = /(?:@@|__)HEW(?:TAG|META)(?:@@|__)/.freeze
  # Tags SketchUp treats as "no tag" — never encoded.
  DEFAULT_TAGS = ["Untagged", "Layer0"].freeze

  module_function

  # Root-first folder path to a tag as an array of segments, e.g.
  # ["Structure", "Roof"]. Flat [name] on pre-2021 SketchUp (no folders).
  def tag_segments(layer)
    segments = [layer.name.to_s]
    # Layer#folder exists from SketchUp 2021; walk up the folder chain.
    if layer.respond_to?(:folder)
      folder = layer.folder
      until folder.nil?
        segments.unshift(folder.name.to_s)
        folder = folder.respond_to?(:folder) ? folder.folder : nil
      end
    end
    segments
  end

  # The human name with any prior HEW payload stripped (idempotent re-encode).
  def base_name(name)
    n = name.to_s
    m = DELIM_RE.match(n)
    m.nil? ? n : n[0...m.begin(0)]
  end

  # "__HEWMETA__<hex>" for a real name + optional list of tag paths. Returns ""
  # when there is nothing worth preserving (empty name, no tags).
  def meta_suffix(real_name, tag_paths)
    return "" if real_name.to_s.empty? && tag_paths.empty?
    payload = { "n" => real_name.to_s }
    payload["t"] = tag_paths unless tag_paths.empty?
    # NOTE: String#unpack1 is Ruby 2.4+; SketchUp's bundled Ruby can be older,
    # so use unpack("H*").first for the lowercase-hex encoding.
    META + JSON.generate(payload).unpack("H*").first
  end

  def tagged?(entity)
    entity.respond_to?(:layer) && !entity.layer.nil? &&
      !DEFAULT_TAGS.include?(entity.layer.name)
  end

  # Visit every Group / ComponentInstance in the model tree (depth-first).
  def each_node(entities, &block)
    entities.each do |e|
      if e.is_a?(Sketchup::Group)
        block.call(e)
        each_node(e.entities, &block)
      elsif e.is_a?(Sketchup::ComponentInstance)
        block.call(e)
        each_node(e.definition.entities, &block)
      end
    end
  end

  # Rewrite an entity/definition/material `name=` to embed its real name + tags.
  # Returns true if it changed anything.
  def encode_name(current, tag_paths)
    real = base_name(current)
    suffix = meta_suffix(real, tag_paths)
    new_name = real + suffix
    new_name == current.to_s ? [false, current.to_s] : [true, new_name]
  end

  def encode
    model = Sketchup.active_model
    count = 0
    model.start_operation("Hew: encode names + tags", true)

    # Group / ComponentInstance nodes — carry name + (per-instance) tag.
    each_node(model.entities) do |entity|
      paths = tagged?(entity) ? [tag_segments(entity.layer)] : []
      changed, new_name = encode_name(entity.name, paths)
      if changed
        entity.name = new_name
        count += 1
      end
    end

    # Component definitions — the def name is what `8' 5/8" Drywall` shows as.
    model.definitions.each do |defn|
      next if defn.image? # skip image definitions
      changed, new_name = encode_name(defn.name, [])
      if changed
        defn.name = new_name
        count += 1
      end
    end

    # Materials — names like `[Metal_Aluminum_Anodized]1`.
    model.materials.each do |mat|
      changed, new_name = encode_name(mat.name, [])
      if changed
        mat.name = new_name
        count += 1
      end
    end

    model.commit_operation
    puts "Hew: encoded #{count} names. Export to COLLADA now, then Hew.decode " \
         "(or Undo) to restore."
    count
  end

  def decode
    model = Sketchup.active_model
    count = 0
    model.start_operation("Hew: decode names + tags", true)

    each_node(model.entities) do |entity|
      next unless DELIM_RE.match?(entity.name.to_s)
      entity.name = base_name(entity.name)
      count += 1
    end
    model.definitions.each do |defn|
      next unless DELIM_RE.match?(defn.name.to_s)
      defn.name = base_name(defn.name)
      count += 1
    end
    model.materials.each do |mat|
      next unless DELIM_RE.match?(mat.name.to_s)
      mat.name = base_name(mat.name)
      count += 1
    end

    model.commit_operation
    puts "Hew: restored #{count} names."
    count
  end

  # Back-compat aliases.
  def encode_tags
    encode
  end

  def decode_tags
    decode
  end
end
