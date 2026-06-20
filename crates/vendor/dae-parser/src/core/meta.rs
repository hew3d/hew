use crate::*;
use chrono::{DateTime, FixedOffset, Local};

/// Collada spec says that `created` and `modified` times should follow ISO 8601,
/// but `chrono` asserts that this means that timezones are required and Blender
/// doesn't seem to add them. To avoid crashing, we store the unparsed date as a string.
#[derive(Clone, Debug)]
pub enum MaybeDateTime {
    /// A proper ISO 8601 date-time.
    Ok(DateTime<FixedOffset>),
    /// A date-time that failed to parse.
    Error(String),
}

impl From<DateTime<FixedOffset>> for MaybeDateTime {
    fn from(v: DateTime<FixedOffset>) -> Self {
        Self::Ok(v)
    }
}

impl FromStr for MaybeDateTime {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.parse() {
            Ok(dt) => Self::Ok(dt),
            Err(_) => Self::Error(s.to_owned()),
        })
    }
}

impl Display for MaybeDateTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaybeDateTime::Ok(dt) => write!(f, "{}", dt.to_rfc3339()),
            MaybeDateTime::Error(s) => write!(f, "{}", s),
        }
    }
}
/// Defines asset-management information regarding its parent element.
#[derive(Clone, Debug)]
pub struct Asset {
    /// Defines data related to a contributor that worked on the parent element.
    pub contributor: Vec<Contributor>,
    /// Contains date and time that the parent element was created.
    /// Represented in an ISO 8601 format as per the XML Schema
    /// `dateTime` primitive type.
    pub created: MaybeDateTime,
    /// Contains a list of words used as search criteria for the parent element.
    pub keywords: Vec<String>,
    /// Contains date and time that the parent element was last
    /// modified. Represented in an ISO 8601 format as per the
    /// XML Schema `dateTime` primitive type.
    pub modified: MaybeDateTime,
    /// Contains revision information for the parent element.
    pub revision: Option<String>,
    /// Contains a description of the topical subject of the parent element.
    pub subject: Option<String>,
    /// Contains title information for the parent element.
    pub title: Option<String>,
    /// Defines unit of distance for COLLADA elements and objects.
    pub unit: Unit,
    /// Contains descriptive information about the coordinate system of the geometric data.
    pub up_axis: UpAxis,
}

impl Asset {
    /// Create a new `Asset` with the given creation and modification dates
    /// and defaulting everything else.
    pub fn new(created: DateTime<FixedOffset>, modified: DateTime<FixedOffset>) -> Self {
        Self {
            contributor: Default::default(),
            created: created.into(),
            keywords: Default::default(),
            modified: modified.into(),
            revision: Default::default(),
            subject: Default::default(),
            title: Default::default(),
            unit: Default::default(),
            up_axis: Default::default(),
        }
    }

    /// Create a new `Asset` object representing an object created at the current date/time.
    pub fn create_now() -> Self {
        let now = Local::now().into();
        Self::new(now, now)
    }
}

impl XNode for Asset {
    const NAME: &'static str = "asset";

    fn parse_box(element: &Element) -> Result<Box<Self>> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Box::new(Asset {
            contributor: Contributor::parse_list(&mut it)?,
            created: parse_one("created", &mut it, parse_elem)?,
            keywords: parse_opt("keywords", &mut it, parse_text)?.map_or_else(Vec::new, |s| {
                s.split_ascii_whitespace().map(|s| s.to_owned()).collect()
            }),
            modified: parse_one("modified", &mut it, parse_elem)?,
            revision: parse_opt("revision", &mut it, parse_text)?,
            subject: parse_opt("subject", &mut it, parse_text)?,
            title: parse_opt("title", &mut it, parse_text)?,
            unit: Unit::parse_opt(&mut it)?.unwrap_or_default(),
            up_axis: UpAxis::parse_opt(&mut it)?.unwrap_or_default(),
        });
        finish(res, it)
    }

    fn parse(element: &Element) -> Result<Self> {
        Ok(*Self::parse_box(element)?)
    }
}

impl XNodeWrite for Asset {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.contributor.write_to(w)?;
        ElemBuilder::print("created", &self.created, w)?;
        if !self.keywords.is_empty() {
            ElemBuilder::print_arr("keywords", &self.keywords, w)?;
        }
        ElemBuilder::print("modified", &self.modified, w)?;
        opt(&self.revision, |e| ElemBuilder::print_str("revision", e, w))?;
        opt(&self.subject, |e| ElemBuilder::print_str("subject", e, w))?;
        opt(&self.title, |e| ElemBuilder::print_str("title", e, w))?;
        if self.unit != Unit::default() {
            self.unit.write_to(w)?;
        }
        if self.up_axis != UpAxis::default() {
            self.up_axis.write_to(w)?;
        }
        e.end(w)
    }
}

/// Defines authoring information for asset management.
#[derive(Clone, Default, Debug)]
pub struct Contributor {
    /// The author’s name
    pub author: Option<String>,
    /// The name of the authoring tool
    pub authoring_tool: Option<String>,
    /// Comments from this contributor
    pub comments: Option<String>,
    /// Copyright information
    pub copyright: Option<String>,
    /// A reference to the source data used for this asset
    pub source_data: Option<Url>,
}

impl Contributor {
    /// Does this element contain no data?
    pub fn is_empty(&self) -> bool {
        self.author.is_none()
            && self.authoring_tool.is_none()
            && self.comments.is_none()
            && self.copyright.is_none()
            && self.source_data.is_none()
    }
}

impl XNode for Contributor {
    const NAME: &'static str = "contributor";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Contributor {
            author: parse_opt("author", &mut it, parse_text)?,
            authoring_tool: parse_opt("authoring_tool", &mut it, parse_text)?,
            comments: parse_opt("comments", &mut it, parse_text)?,
            copyright: parse_opt("copyright", &mut it, parse_text)?,
            source_data: parse_opt("source_data", &mut it, parse_elem)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for Contributor {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem();
        if self.is_empty() {
            e.end(w)
        } else {
            let e = e.start(w)?;
            opt(&self.author, |e| ElemBuilder::print_str("author", e, w))?;
            opt(&self.authoring_tool, |e| {
                ElemBuilder::print_str("authoring_tool", e, w)
            })?;
            opt(&self.comments, |e| ElemBuilder::print_str("comments", e, w))?;
            opt(&self.copyright, |e| {
                ElemBuilder::print_str("copyright", e, w)
            })?;
            opt(&self.source_data, |e| {
                ElemBuilder::print("source_data", e, w)
            })?;
            e.end(w)
        }
    }
}

/// Defines unit of distance for COLLADA elements and objects.
/// This unit of distance applies to all spatial measurements
/// within the scope of `Asset`’s parent element, unless
/// overridden by a more local `Asset` / `Unit`.
///
/// The value of the unit is self-describing and does not have to be consistent
/// with any real-world measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    /// The name of the distance unit. For example, "meter", "centimeter", "inches",
    /// or "parsec". This can be the real name of a measurement, or an imaginary name.
    pub name: Option<String>,
    /// How many real-world meters in one
    /// distance unit as a floating-point number. For
    /// example, 1.0 for the name "meter"; 1000 for the
    /// name "kilometer"; 0.3048 for the name "foot".
    pub meter: f32,
}

impl Default for Unit {
    fn default() -> Self {
        Unit {
            name: None,
            meter: 1.,
        }
    }
}

impl XNode for Unit {
    const NAME: &'static str = "unit";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Unit {
            name: match element.attr("name") {
                None | Some("meter") => None,
                Some(name) => Some(name.into()),
            },
            meter: match element.attr("meter") {
                None => 1.,
                Some(s) => s.parse().map_err(|_| "parse error")?,
            },
        })
    }
}

impl XNodeWrite for Unit {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("name", &self.name);
        e.def_print_attr("meter", self.meter, 1.);
        e.end(w)
    }
}

/// Descriptive information about the coordinate system of the geometric data.
/// All coordinates are right-handed by definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpAxis {
    /// Right: `-y`, Up: `+x`, In: `+z`
    XUp,
    /// Right: `+x`, Up: `+y`, In: `+z`
    YUp,
    /// Right: `+x`, Up: `+z`, In: `-y`
    ZUp,
}

impl Default for UpAxis {
    fn default() -> Self {
        Self::YUp
    }
}

impl UpAxis {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::XUp => "X_UP",
            Self::YUp => "Y_UP",
            Self::ZUp => "Z_UP",
        }
    }
}

impl Display for UpAxis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

impl XNode for UpAxis {
    const NAME: &'static str = "up_axis";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(match get_text(element) {
            Some("X_UP") => UpAxis::XUp,
            Some("Y_UP") => UpAxis::YUp,
            Some("Z_UP") => UpAxis::ZUp,
            _ => return Err("invalid <up_axis> value".into()),
        })
    }
}

impl XNodeWrite for UpAxis {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        ElemBuilder::print_str(Self::NAME, self.to_str(), w)
    }
}
