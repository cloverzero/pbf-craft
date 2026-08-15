//! Element models for OpenStreetMap data.
//!
//! The three OSM element types are [`crate::models::Node`], [`crate::models::Way`] and [`crate::models::Relation`], which all share the
//! common metadata fields of [`crate::models::ElementBase`] (id, version, timestamp, user, changeset id,
//! visible flag and tags) and are carried polymorphically by the [`crate::models::Element`] enum.
//!
//! # Units
//!
//! Coordinates are stored as **integer nanodegrees** — the raw unit used by the PBF format
//! (1e9 nanodegrees = 1 degree). This avoids floating-point precision loss on round-trips.
//! Divide by `1e9` to obtain degrees. [`crate::models::Bound`] fields use the same unit.
//!
//! # The `visible` flag and metadata defaults
//!
//! Per the PBF spec the `visible` flag is assumed `true` when absent. All element types
//! therefore default `visible` to `true`, and `timestamp`/`user` are `Option`s that are
//! `None` when the source data carries no such metadata. `version`/`changeset_id` default to
//! `-1` (the convention used by osmosis for "no version"/"no changeset").
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A bounding box from the PBF file header.
///
/// Coordinates are in integer **nanodegrees** (1e9 per degree). `origin` is the data source
/// string recorded in the header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bound {
    pub left: i64,
    pub right: i64,
    pub top: i64,
    pub bottom: i64,
    pub origin: String,
}

/// The user associated with an element (a mapper account name and id).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OsmUser {
    pub id: i32,
    pub name: String,
}

/// A polymorphic OSM element: either a [`Node`], a [`Way`] or a [`Relation`].
///
/// Serialized with a `type` tag (`"node"`, `"way"`, `"relation"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Element {
    Node(Node),
    Way(Way),
    Relation(Relation),
}

impl Element {
    /// Returns the element's `(type, id)` pair.
    pub fn get_meta(&self) -> (ElementType, i64) {
        match self {
            Element::Node(e) => (ElementType::Node, e.id),
            Element::Way(e) => (ElementType::Way, e.id),
            Element::Relation(e) => (ElementType::Relation, e.id),
        }
    }
}

/// The type of an OSM element.
///
/// Can be parsed from the lowercase strings `"node"`, `"way"` and `"relation"` via
/// [`FromStr`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ElementType {
    Node,
    Way,
    Relation,
}

impl FromStr for ElementType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "node" => Ok(ElementType::Node),
            "way" => Ok(ElementType::Way),
            "relation" => Ok(ElementType::Relation),
            _ => Err(anyhow!("Illegal element_type: {}", s)),
        }
    }
}

/// Common metadata shared by [`Node`], [`Way`] and [`Relation`].
///
/// See the [module docs](self) for the default values (`visible = true`,
/// `version = changeset_id = -1`, `timestamp`/`user` = `None`).
#[derive(Debug)]
pub struct ElementBase {
    pub id: i64,
    pub version: i32,
    pub timestamp: Option<DateTime<Utc>>,
    pub user: Option<OsmUser>,
    pub changeset_id: i64,
    pub visible: bool,
    pub tags: Vec<Tag>,
}

// `visible` defaults to true: the PBF spec states the flag "MUST be assumed to be true" when
// absent, and a derived `Default` would yield `false` for the `bool`, silently marking every
// freshly-created element as deleted on write.
impl Default for ElementBase {
    fn default() -> Self {
        Self {
            id: 0,
            version: -1,
            timestamp: None,
            user: None,
            changeset_id: -1,
            visible: true,
            tags: Vec::new(),
        }
    }
}

impl ElementBase {
    /// Creates base metadata for an element with only an id and tags (no version, timestamp
    /// or user information).
    pub fn new_with_tags(id: i64, tags: Vec<Tag>) -> Self {
        Self {
            id,
            tags,
            visible: true,
            ..Default::default()
        }
    }
}

/// A `key=value` pair attached to an element.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tag {
    pub key: String,
    pub value: String,
}

/// An OSM node: a point with a coordinate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    pub id: i64,
    pub version: i32,
    pub timestamp: Option<DateTime<Utc>>,
    pub user: Option<OsmUser>,
    pub changeset_id: i64,
    /// Latitude in integer **nanodegrees** (divide by 1e9 for degrees).
    pub latitude: i64,
    /// Longitude in integer **nanodegrees** (divide by 1e9 for degrees).
    pub longitude: i64,
    /// `false` marks a deleted/historical object; defaults to `true` (see module docs).
    pub visible: bool,
    pub tags: Vec<Tag>,
}

// See the comment on `ElementBase::default()`: `visible` must default to true, not to the
// derived `bool` default of false.
impl Default for Node {
    fn default() -> Self {
        Self {
            id: 0,
            version: -1,
            timestamp: None,
            user: None,
            changeset_id: -1,
            latitude: 0,
            longitude: 0,
            visible: true,
            tags: Vec::new(),
        }
    }
}

impl From<ElementBase> for Node {
    fn from(el: ElementBase) -> Self {
        Self {
            id: el.id,
            version: el.version,
            timestamp: el.timestamp,
            user: el.user,
            changeset_id: el.changeset_id,
            visible: el.visible,
            tags: el.tags,
            latitude: 0,
            longitude: 0,
        }
    }
}

/// An OSM way: an ordered list of node references ([`WayNode`]s).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Way {
    pub id: i64,
    pub version: i32,
    pub timestamp: Option<DateTime<Utc>>,
    pub user: Option<OsmUser>,
    pub changeset_id: i64,
    /// `false` marks a deleted/historical object; defaults to `true` (see module docs).
    pub visible: bool,
    pub tags: Vec<Tag>,
    /// The way's nodes in order. Coordinates are present only when the file declares the
    /// `LocationsOnWays` feature.
    pub way_nodes: Vec<WayNode>,
}

// `visible` defaults to true — see `ElementBase::default()`.
impl Default for Way {
    fn default() -> Self {
        Self {
            id: 0,
            version: -1,
            timestamp: None,
            user: None,
            changeset_id: -1,
            visible: true,
            tags: Vec::new(),
            way_nodes: Vec::new(),
        }
    }
}

impl From<ElementBase> for Way {
    fn from(el: ElementBase) -> Self {
        Self {
            id: el.id,
            version: el.version,
            timestamp: el.timestamp,
            user: el.user,
            changeset_id: el.changeset_id,
            visible: el.visible,
            tags: el.tags,
            way_nodes: Vec::new(),
        }
    }
}

/// A reference to a node within a [`Way`], optionally carrying the node's coordinates.
///
/// Coordinates are in integer **nanodegrees** and are only populated when the PBF file
/// carries node locations on ways (`LocationsOnWays` optional feature).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WayNode {
    /// The referenced node's id.
    pub id: i64,
    pub latitude: Option<i64>,
    pub longitude: Option<i64>,
}

impl WayNode {
    /// Creates a node reference without coordinates.
    pub fn new_without_coords(id: i64) -> Self {
        Self {
            id,
            latitude: None,
            longitude: None,
        }
    }

    /// Creates a node reference with coordinates (in integer nanodegrees).
    pub fn new(id: i64, latitude: i64, longitude: i64) -> Self {
        Self {
            id,
            latitude: Some(latitude),
            longitude: Some(longitude),
        }
    }
}

/// An OSM relation: a set of typed member references ([`RelationMember`]s).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Relation {
    pub id: i64,
    pub version: i32,
    pub timestamp: Option<DateTime<Utc>>,
    pub user: Option<OsmUser>,
    pub changeset_id: i64,
    /// `false` marks a deleted/historical object; defaults to `true` (see module docs).
    pub visible: bool,
    pub tags: Vec<Tag>,
    pub members: Vec<RelationMember>,
}

// `visible` defaults to true — see `ElementBase::default()`.
impl Default for Relation {
    fn default() -> Self {
        Self {
            id: 0,
            version: -1,
            timestamp: None,
            user: None,
            changeset_id: -1,
            visible: true,
            tags: Vec::new(),
            members: Vec::new(),
        }
    }
}

impl From<ElementBase> for Relation {
    fn from(el: ElementBase) -> Self {
        Self {
            id: el.id,
            version: el.version,
            timestamp: el.timestamp,
            user: el.user,
            changeset_id: el.changeset_id,
            visible: el.visible,
            tags: el.tags,
            members: Vec::new(),
        }
    }
}

/// A member of a [`Relation`]: a typed reference to another element plus a role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationMember {
    /// The referenced element's id.
    pub member_id: i64,
    /// The referenced element's type.
    pub member_type: ElementType,
    /// The member's role within the relation (e.g. `"outer"`, `"inner"`).
    pub role: String,
}

/// Common accessors implemented by [`Node`], [`Way`] and [`Relation`].
pub trait BasicElement: Clone {
    fn get_element_type() -> ElementType;
    fn get_id(&self) -> i64;
    fn get_version(&self) -> i32;
    fn get_timestamp(&self) -> Option<DateTime<Utc>>;
    fn get_changeset_id(&self) -> i64;
    fn is_visible(&self) -> bool;
    fn get_tags(&self) -> &Vec<Tag>;
    fn get_user(&self) -> Option<&OsmUser>;
}

impl BasicElement for Node {
    fn get_element_type() -> ElementType {
        ElementType::Node
    }

    fn get_id(&self) -> i64 {
        self.id
    }

    fn get_version(&self) -> i32 {
        self.version
    }

    fn get_timestamp(&self) -> Option<DateTime<Utc>> {
        self.timestamp
    }

    fn get_changeset_id(&self) -> i64 {
        self.changeset_id
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn get_tags(&self) -> &Vec<Tag> {
        &self.tags
    }

    fn get_user(&self) -> Option<&OsmUser> {
        self.user.as_ref()
    }
}

impl BasicElement for Way {
    fn get_element_type() -> ElementType {
        ElementType::Way
    }

    fn get_id(&self) -> i64 {
        self.id
    }

    fn get_version(&self) -> i32 {
        self.version
    }

    fn get_timestamp(&self) -> Option<DateTime<Utc>> {
        self.timestamp
    }

    fn get_changeset_id(&self) -> i64 {
        self.changeset_id
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn get_tags(&self) -> &Vec<Tag> {
        &self.tags
    }

    fn get_user(&self) -> Option<&OsmUser> {
        self.user.as_ref()
    }
}

impl BasicElement for Relation {
    fn get_element_type() -> ElementType {
        ElementType::Relation
    }

    fn get_id(&self) -> i64 {
        self.id
    }

    fn get_version(&self) -> i32 {
        self.version
    }

    fn get_timestamp(&self) -> Option<DateTime<Utc>> {
        self.timestamp
    }

    fn get_changeset_id(&self) -> i64 {
        self.changeset_id
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn get_tags(&self) -> &Vec<Tag> {
        &self.tags
    }

    fn get_user(&self) -> Option<&OsmUser> {
        self.user.as_ref()
    }
}
