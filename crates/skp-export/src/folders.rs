//! Tag hierarchy in the 2026 container: a tag path with more than one
//! segment is written as layer folders (every segment but the last) and
//! a layer named by the last segment; the stream containers have no
//! folders and keep one flat layer per tag, named by the joined path.
//! Also the pages' own timing and hidden folders (`PageExtras`).

use std::collections::{BTreeMap, BTreeSet};

use skpwright::{LayerFolder, Model, PageExtras};

use crate::Author;

/// Every folder's tag path in pre-order, before and after a save.
pub(crate) type FolderPaths = (Vec<Vec<String>>, Vec<Vec<String>>);

/// What one document scene adds to its page record.
pub(crate) struct PagePlan {
    pub transition_s: Option<f64>,
    pub delay_s: Option<f64>,
    /// The tag paths the scene hides (`None` = it captures none).
    pub hidden: Option<Vec<Vec<String>>>,
}

/// The tag-path segment of each folder of a list (see
/// [`skp_import::folder_segments`]): sibling folders of one name stay
/// distinct tags.
fn segments(list: &[LayerFolder]) -> Vec<String> {
    skp_import::folder_segments(&list.iter().map(|f| f.name.as_str()).collect::<Vec<_>>())
}

/// The folders of a model as tag paths: each layer's path (folders'
/// names, then its own), and every folder's path in pre-order.
pub(crate) fn source_paths(model: &Model) -> (BTreeMap<String, Vec<String>>, Vec<Vec<String>>) {
    fn walk(
        model: &Model,
        list: &[LayerFolder],
        prefix: &[String],
        layers: &mut BTreeMap<String, Vec<String>>,
        pre: &mut Vec<Vec<String>>,
    ) {
        for (f, seg) in list.iter().zip(segments(list)) {
            let mut path = prefix.to_vec();
            path.push(seg);
            pre.push(path.clone());
            for &i in &f.layers {
                if let Some(l) = model.layers.get(i).filter(|_| i > 0)
                    && !l.name.is_empty()
                {
                    let mut lp = path.clone();
                    lp.push(l.name.clone());
                    layers.entry(l.name.clone()).or_insert(lp);
                }
            }
            walk(model, &f.folders, &path, layers, pre);
        }
    }
    let mut layers = BTreeMap::new();
    let mut pre = Vec::new();
    walk(
        model,
        &model.features.layer_folders,
        &[],
        &mut layers,
        &mut pre,
    );
    (layers, pre)
}

/// The slots of the layers any entity of the model is on.
pub(crate) fn used_slots(m: &Model) -> BTreeSet<u16> {
    let mut used: BTreeSet<u16> = BTreeSet::new();
    for r in &m.geometry {
        used.extend(r.mesh.faces.iter().map(|f| f.layer));
        used.extend(r.mesh.edges.iter().map(|e| e.layer));
        used.extend(r.placed.iter().map(|p| p.layer));
        used.extend(r.sections.iter().map(|s| s.layer));
        used.extend(r.guide_points.iter().map(|g| g.layer));
        used.extend(r.polylines.iter().map(|p| p.layer));
        used.extend(r.images.iter().map(|im| im.layer));
    }
    used.extend(m.texts.iter().map(|t| t.layer));
    used.extend(m.dimensions.iter().map(|d| d.layer));
    used.extend(m.radial_dimensions.iter().map(|d| d.layer));
    used.extend(m.guides.iter().map(|g| g.layer));
    used
}

impl Author<'_> {
    /// The name a tag's new layer takes: the last segment in a container
    /// with folders, the joined path in one without.
    pub(crate) fn layer_name_for(&self, path: &[String]) -> String {
        match (self.folders, path.last()) {
            (true, Some(last)) => last.clone(),
            _ => path.join("/"),
        }
    }

    /// Whether `path` is a folder: the parent of another tag, or a folder
    /// of the source file.
    fn is_folder(&self, path: &[String]) -> bool {
        self.src_folder_paths.iter().any(|p| p == path)
            || self
                .doc
                .tag_meta()
                .any(|(p, _)| p.len() > path.len() && p.starts_with(path))
    }

    /// A tag that is a folder and got a layer only because every tag does
    /// gives it up when nothing is on it, so the folder stands alone.
    pub(crate) fn release_folder_layers(&mut self) {
        let used = used_slots(&self.model);
        let release: Vec<Vec<String>> = self
            .new_tag_layers
            .iter()
            .filter(|p| self.is_folder(p))
            .filter(|p| self.layer_slot.get(*p).is_some_and(|s| !used.contains(s)))
            .cloned()
            .collect();
        for p in release {
            if let Some(slot) = self.layer_slot.remove(&p)
                && let Some(i) = self
                    .model
                    .layer_links
                    .iter()
                    .find(|l| l.0 == slot)
                    .map(|l| l.1)
            {
                // The tag's attributes go on its folder (`finish_folders`).
                let mut taken = Vec::new();
                self.model.dictionaries.retain(|a| {
                    if a.owner == skpwright::Owner::Layer(i) {
                        taken.extend(a.dictionaries.iter().cloned());
                        false
                    } else {
                        true
                    }
                });
                if !taken.is_empty() {
                    self.released_attrs.push((p, taken));
                }
            }
        }
    }

    /// Writes the folder tree the tags' paths spell (after the layers
    /// are final), keeping the file's own folder fields.
    pub(crate) fn finish_folders(&mut self) {
        let index_of =
            |m: &Model, slot: u16| m.layer_links.iter().find(|l| l.0 == slot).map(|l| l.1);
        let mut layers: Vec<(Vec<String>, usize)> = self
            .layer_slot
            .iter()
            .filter_map(|(p, &s)| Some((p.clone(), index_of(&self.model, s).filter(|&i| i > 0)?)))
            .collect();
        let mut folders: BTreeSet<Vec<String>> = BTreeSet::new();
        for (p, _) in &layers {
            for n in 1..p.len() {
                folders.insert(p[..n].to_vec());
            }
        }
        for (p, _) in self.doc.tag_meta() {
            let has_layer = self.layer_slot.contains_key(p);
            let nested = self
                .doc
                .tag_meta()
                .any(|(q, _)| q.len() > p.len() && q.starts_with(p));
            let was_folder = self.src_folder_paths.iter().any(|f| f == p);
            // A folder of the file stays even when a layer of the same
            // name stands on its tag path (the two are one tag in Hew).
            if !p.is_empty()
                && ((!has_layer && (nested || was_folder)) || (has_layer && was_folder))
            {
                folders.insert(p.to_vec());
                for n in 1..p.len() {
                    folders.insert(p[..n].to_vec());
                }
            }
        }
        // The file's folders by path: the fields they keep, and the order
        // of their layers and subfolders.
        fn old_walk<'a>(
            list: &'a [LayerFolder],
            prefix: &[String],
            out: &mut BTreeMap<Vec<String>, &'a LayerFolder>,
        ) {
            for (f, seg) in list.iter().zip(segments(list)) {
                let mut p = prefix.to_vec();
                p.push(seg);
                out.insert(p.clone(), f);
                old_walk(&f.folders, &p, out);
            }
        }
        let old_tree = std::mem::take(&mut self.model.features.layer_folders);
        let mut old: BTreeMap<Vec<String>, &LayerFolder> = BTreeMap::new();
        old_walk(&old_tree, &[], &mut old);
        // A layer of the file no tag stands for, kept because content the
        // document never read in is on it, stays in its folder.
        let claimed: BTreeSet<usize> = layers.iter().map(|(_, i)| *i).collect();
        for (p, f) in &old {
            for &i in &f.layers {
                if i > 0
                    && !claimed.contains(&i)
                    && let Some(l) = self.model.layers.get(i)
                {
                    let mut lp = p.clone();
                    lp.push(l.name.clone());
                    layers.push((lp, i));
                    for n in 1..=p.len() {
                        folders.insert(p[..n].to_vec());
                    }
                }
            }
        }
        let mut old_pre = Vec::new();
        fn pre_walk(list: &[LayerFolder], prefix: &[String], out: &mut Vec<Vec<String>>) {
            for (f, seg) in list.iter().zip(segments(list)) {
                let mut p = prefix.to_vec();
                p.push(seg);
                out.push(p.clone());
                pre_walk(&f.folders, &p, out);
            }
        }
        pre_walk(&old_tree, &[], &mut old_pre);
        fn build(
            prefix: &[String],
            folders: &BTreeSet<Vec<String>>,
            layers: &[(Vec<String>, usize)],
            old: &BTreeMap<Vec<String>, &LayerFolder>,
            old_children: &[String],
            hidden: &dyn Fn(&[String]) -> bool,
            pre: &mut Vec<Vec<String>>,
        ) -> Vec<LayerFolder> {
            let mut names: Vec<String> = Vec::new();
            for n in old_children {
                let mut p = prefix.to_vec();
                p.push(n.clone());
                if folders.contains(&p) && !names.contains(n) {
                    names.push(n.clone());
                }
            }
            for p in folders {
                if p.len() == prefix.len() + 1
                    && p.starts_with(prefix)
                    && !names.contains(&p[prefix.len()])
                {
                    names.push(p[prefix.len()].clone());
                }
            }
            names
                .into_iter()
                .map(|name| {
                    let mut path = prefix.to_vec();
                    path.push(name.clone());
                    pre.push(path.clone());
                    let was = old.get(&path).copied();
                    let mut mine: Vec<usize> = layers
                        .iter()
                        .filter(|(p, _)| p.len() == path.len() + 1 && p.starts_with(&path))
                        .map(|(_, i)| *i)
                        .collect();
                    mine.sort();
                    let mut ordered: Vec<usize> = was
                        .map(|f| {
                            f.layers
                                .iter()
                                .copied()
                                .filter(|i| mine.contains(i))
                                .collect()
                        })
                        .unwrap_or_default();
                    let rest: Vec<usize> =
                        mine.into_iter().filter(|i| !ordered.contains(i)).collect();
                    ordered.extend(rest);
                    let children: Vec<String> =
                        was.map(|f| segments(&f.folders)).unwrap_or_default();
                    let sub = build(&path, folders, layers, old, &children, hidden, pre);
                    LayerFolder {
                        // The file's own name for a folder it has (a
                        // sibling of one name is a tag `Name (2)`).
                        name: was.map_or(name, |f| f.name.clone()),
                        hidden: hidden(&path),
                        hidden_on_new_pages: was.is_some_and(|f| f.hidden_on_new_pages),
                        layers: ordered,
                        folders: sub,
                    }
                })
                .collect()
        }
        let top: Vec<String> = segments(&old_tree);
        let doc = self.doc;
        let mut new_pre = Vec::new();
        let tree = build(
            &[],
            &folders,
            &layers,
            &old,
            &top,
            &|p| doc.tag_hidden(p),
            &mut new_pre,
        );
        self.model.features.layer_folders = tree;
        // The folders' attributes are keyed by pre-order position, which
        // an added or removed folder moves: each follows its folder by
        // path, and goes with a folder that is gone.
        let moved = |o: skpwright::Owner| match o {
            skpwright::Owner::LayerFolder(i) => old_pre
                .get(i)
                .and_then(|p| new_pre.iter().position(|q| q == p))
                .map(skpwright::Owner::LayerFolder),
            other => Some(other),
        };
        let dictionaries = std::mem::take(&mut self.model.dictionaries);
        self.model.dictionaries = dictionaries
            .into_iter()
            .filter_map(|mut a| {
                a.owner = moved(a.owner)?;
                Some(a)
            })
            .collect();
        for (p, dicts) in std::mem::take(&mut self.released_attrs) {
            if let Some(i) = new_pre.iter().position(|q| *q == p) {
                let owner = skpwright::Owner::LayerFolder(i);
                // The tag's dictionaries go over the same-named ones of the
                // folder's own; the folder's others stay.
                let mut merged: Vec<skpwright::Dictionary> = self
                    .model
                    .dictionaries
                    .iter()
                    .filter(|a| a.owner == owner)
                    .flat_map(|a| a.dictionaries.iter().cloned())
                    .filter(|d| !dicts.iter().any(|n| n.name == d.name))
                    .collect();
                merged.extend(dicts);
                self.model.dictionaries.retain(|a| a.owner != owner);
                self.model.dictionaries.push(skpwright::Attributes {
                    owner,
                    dictionaries: merged,
                });
            }
        }
        self.folder_paths = Some((old_pre, new_pre));
    }

    /// Writes each scene's own timing and hidden folders into its page
    /// record, and moves the file's other page records' folder indices
    /// onto the folders as they are now.
    pub(crate) fn finish_pages(&mut self) {
        let plans = std::mem::take(&mut self.page_plans);
        let paths = self.folder_paths.take();
        let mut pages: BTreeMap<usize, PageExtras> =
            std::mem::take(&mut self.model.features.scene_pages)
                .into_iter()
                .collect();
        if let Some((old_pre, new_pre)) = &paths {
            for e in pages.values_mut() {
                e.hidden_layer_folders = e
                    .hidden_layer_folders
                    .iter()
                    .filter_map(|&i| old_pre.get(i))
                    .filter_map(|p| new_pre.iter().position(|q| q == p))
                    .collect();
            }
        }
        // The plans are in the order the scenes were written.
        for (k, plan) in plans.into_iter().enumerate() {
            if k >= self.model.scenes.len() {
                continue;
            }
            let e = pages.entry(k).or_default();
            e.transition_s = plan.transition_s;
            e.delay_s = plan.delay_s;
            if let (Some(hidden), Some((_, new_pre))) = (&plan.hidden, &paths) {
                e.hidden_layer_folders = new_pre
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| hidden.iter().any(|h| f.starts_with(h)))
                    .map(|(i, _)| i)
                    .collect();
            }
        }
        // A page lists hidden folders only while it saves its visible
        // layers (a scene that stopped capturing its tags saves none).
        for (k, e) in pages.iter_mut() {
            let saves = self
                .model
                .scenes
                .get(*k)
                .is_some_and(|s| s.saved.has(skpwright::SceneProperties::VISIBLE_LAYERS));
            if !saves {
                e.hidden_layer_folders.clear();
            }
        }
        self.model.features.scene_pages = pages
            .into_iter()
            .filter(|(_, e)| *e != PageExtras::default())
            .collect();
    }
}
