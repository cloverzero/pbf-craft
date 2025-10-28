use crate::db::paging_cursor::PagingCursor;
use chrono::{DateTime, NaiveDateTime, Utc};
use pbf_craft::models::{
    Element, ElementType, Node, OsmUser, Relation, RelationMember, Tag, Way, WayNode,
};
use postgres::config::Config;
use postgres::NoTls;
use postgres_types::{FromSql, ToSql};

pub struct DatabaseReader {
    config: Config,
}

#[derive(Debug, ToSql, FromSql)]
#[postgres(name = "nwr_enum")]
pub enum DbElementType {
    #[postgres(name = "Node")]
    Node,
    #[postgres(name = "Way")]
    Way,
    #[postgres(name = "Relation")]
    Relation,
}

// impl Into<ElementType> for DbElementType {
//     fn into(self) -> ElementType {
//         match self {
//             DbElementType::Node => ElementType::Node,
//             DbElementType::Way => ElementType::Way,
//             DbElementType::Relation => ElementType::Relation,
//         }
//     }
// }

impl From<DbElementType> for ElementType {
    fn from(value: DbElementType) -> Self {
        match value {
            DbElementType::Node => ElementType::Node,
            DbElementType::Way => ElementType::Way,
            DbElementType::Relation => ElementType::Relation,
        }
    }
}

impl DatabaseReader {
    pub fn new(host: String, port: u16, dbname: String, user: String, password: String) -> Self {
        let mut config = Config::new();
        let _ = config
            .host(&host)
            .port(port)
            .dbname(&dbname)
            .user(&user)
            .password(&password);
        Self { config }
    }

    pub fn read<F>(&self, mut callback: F) -> anyhow::Result<()>
    where
        F: FnMut(Element),
    {
        blue_ln!("Exporting nodes ...");
        self.read_nodes(&mut callback)?;
        blue_ln!("Exporting ways ...");
        self.read_ways(&mut callback)?;
        blue_ln!("Exporting relations ...");
        self.read_relations(&mut callback)?;

        Ok(())
    }

    fn read_nodes<F>(&self, callback: &mut F) -> anyhow::Result<()>
    where
        F: FnMut(Element),
    {
        let mut el_client = self.config.connect(NoTls)?;
        let node_cursor = PagingCursor::new(
            "SELECT e.id, e.latitude, e.longitude, e.changeset_id, e.timestamp, e.\"version\", e.visible, \
            u.id as user_id, u.display_name \
            FROM current_nodes e \
            LEFT JOIN changesets c ON e.changeset_id = c.id \
            LEFT JOIN users u ON c.user_id = u.id \
            WHERE e.visible = true \
            ORDER BY id",
            &mut el_client,
        );

        let mut tag_client = self.config.connect(NoTls)?;
        let mut tag_iter = PagingCursor::new(
            "SELECT node_id, k, v FROM current_node_tags ORDER BY node_id",
            &mut tag_client,
        );

        let mut current_tag_id = 0;
        let mut current_tag: Option<Tag> = None;
        for node_row in node_cursor {
            let mut node = Node {
                id: node_row.get(0),
                latitude: node_row.get::<_, i32>(1) as i64 * 100,
                longitude: node_row.get::<_, i32>(2) as i64 * 100,
                changeset_id: node_row.get(3),
                timestamp: Some(DateTime::from_naive_utc_and_offset(node_row.get(4), Utc)),
                version: node_row.get::<_, i64>(5) as i32,
                visible: node_row.get(6),
                user: Some(OsmUser {
                    id: node_row.get::<_, i64>(7) as i32,
                    name: node_row.get(8),
                }),
                ..Default::default()
            };

            if node.id == current_tag_id && current_tag.is_some() {
                node.tags.push(current_tag.unwrap());
                current_tag = None;
            }
            while current_tag_id <= node.id || current_tag.is_none() {
                let has_tag = tag_iter.next();
                if has_tag.is_none() {
                    break;
                }
                let tag_row = has_tag.unwrap();
                current_tag_id = tag_row.get(0);
                let key: String = tag_row.get(1);
                let value: String = tag_row.get(2);
                let tag = Tag { key, value };
                if current_tag_id == node.id {
                    node.tags.push(tag);
                } else {
                    current_tag = Some(tag);
                }
            }
            let el = Element::Node(node);
            callback(el)
        }

        Ok(())
    }

    fn read_ways<F>(&self, callback: &mut F) -> anyhow::Result<()>
    where
        F: FnMut(Element),
    {
        let mut el_client = self.config.connect(NoTls)?;
        let el_cursor = PagingCursor::new(
            "SELECT e.id, e.changeset_id, e.timestamp, e.\"version\", e.visible, \
            u.id as user_id, u.display_name \
            FROM current_ways e \
            INNER JOIN changesets c ON e.changeset_id = c.id \
            INNER JOIN users u ON c.user_id = u.id \
            WHERE e.visible = true \
            ORDER BY id",
            &mut el_client,
        );

        let mut tag_client = self.config.connect(NoTls)?;
        let mut tag_iter = PagingCursor::new(
            "SELECT way_id, k, v FROM current_way_tags ORDER BY way_id",
            &mut tag_client,
        );

        let mut mem_client = self.config.connect(NoTls)?;
        let mut member_iter = PagingCursor::new(
            "SELECT way_id, node_id, sequence_id FROM current_way_nodes ORDER BY way_id, sequence_id",
            &mut mem_client,
        );

        let mut current_tag_id = 0;
        let mut current_tag: Option<Tag> = None;
        let mut current_mem_id = 0;
        let mut current_mem: Option<WayNode> = None;
        for el_row in el_cursor {
            let mut way = Way {
                id: el_row.get(0),
                changeset_id: el_row.get(1),
                timestamp: Some(DateTime::from_naive_utc_and_offset(el_row.get(2), Utc)),
                version: el_row.get::<_, i64>(3) as i32,
                visible: el_row.get(4),
                user: Some(OsmUser {
                    id: el_row.get::<_, i64>(5) as i32,
                    name: el_row.get(6),
                }),
                ..Default::default()
            };

            if current_tag_id == way.id && current_tag.is_some() {
                way.tags.push(current_tag.unwrap());
                current_tag = None;
            }
            while current_tag_id <= way.id || current_tag.is_none() {
                let has_tag = tag_iter.next();
                if has_tag.is_none() {
                    break;
                }
                let tag_row = has_tag.unwrap();
                current_tag_id = tag_row.get(0);
                let key: String = tag_row.get(1);
                let value: String = tag_row.get(2);
                let tag = Tag { key, value };
                if current_tag_id == way.id {
                    way.tags.push(tag);
                } else {
                    current_tag = Some(tag);
                }
            }

            if way.id == current_mem_id && current_mem.is_some() {
                way.way_nodes.push(current_mem.unwrap());
                current_mem = None;
            }
            while current_mem_id <= way.id || current_mem.is_none() {
                let has_mem = member_iter.next();
                if has_mem.is_none() {
                    break;
                }
                let mem_row = has_mem.unwrap();
                current_mem_id = mem_row.get(0);
                let node_id: i64 = mem_row.get(1);
                //                let seq_id: i64 = mem_row.get(3);
                let way_node = WayNode {
                    id: node_id,
                    latitude: None,
                    longitude: None,
                };
                if current_mem_id == way.id {
                    way.way_nodes.push(way_node);
                } else {
                    current_mem = Some(way_node);
                }
            }

            let el = Element::Way(way);
            callback(el)
        }

        Ok(())
    }

    fn read_relations<F>(&self, callback: &mut F) -> anyhow::Result<()>
    where
        F: FnMut(Element),
    {
        let mut el_client = self.config.connect(NoTls)?;
        let el_cursor = PagingCursor::new(
            "SELECT e.id, e.changeset_id, e.timestamp, e.\"version\", e.visible, \
                u.id as user_id, u.display_name \
                FROM current_relations e \
                INNER JOIN changesets c ON e.changeset_id = c.id \
                INNER JOIN users u ON c.user_id = u.id \
                WHERE e.visible = true \
                ORDER BY id",
            &mut el_client,
        );

        let mut tag_client = self.config.connect(NoTls)?;
        let mut tag_iter = PagingCursor::new(
            "SELECT relation_id, k, v FROM current_relation_tags ORDER BY relation_id",
            &mut tag_client,
        );

        let mut mem_client = self.config.connect(NoTls)?;
        let mut member_iter = PagingCursor::new(
                "SELECT relation_id, member_type, member_id, member_role FROM current_relation_members ORDER BY relation_id, sequence_id",
            &mut mem_client
        );

        let mut current_tag_id = 0;
        let mut current_tag: Option<Tag> = None;
        let mut current_mem_id = 0;
        let mut current_mem: Option<RelationMember> = None;
        for el_row in el_cursor {
            let timestamp: NaiveDateTime = el_row.get(2);
            let utc_timestamp: DateTime<Utc> = DateTime::from_naive_utc_and_offset(timestamp, Utc);
            let mut relation = Relation {
                id: el_row.get(0),
                changeset_id: el_row.get(1),
                timestamp: Some(utc_timestamp),
                version: el_row.get::<_, i64>(3) as i32,
                visible: el_row.get(4),
                user: Some(OsmUser {
                    id: el_row.get::<_, i64>(5) as i32,
                    name: el_row.get(6),
                }),
                ..Default::default()
            };

            if relation.id == current_tag_id && current_tag.is_some() {
                relation.tags.push(current_tag.unwrap());
                current_tag = None;
            }
            while current_tag_id <= relation.id || current_tag.is_none() {
                let has_tag = tag_iter.next();
                if has_tag.is_none() {
                    break;
                }
                let tag_row = has_tag.unwrap();
                current_tag_id = tag_row.get(0);
                let key: String = tag_row.get(1);
                let value: String = tag_row.get(2);
                let tag = Tag { key, value };
                if current_tag_id == relation.id {
                    relation.tags.push(tag);
                } else {
                    current_tag = Some(tag);
                }
            }

            if relation.id == current_mem_id && current_mem.is_some() {
                relation.members.push(current_mem.unwrap());
                current_mem = None;
            }
            while current_mem_id <= relation.id || current_mem.is_none() {
                let has_mem = member_iter.next();
                if has_mem.is_none() {
                    break;
                }
                let mem_row = has_mem.unwrap();
                current_mem_id = mem_row.get(0);
                let db_member_type: DbElementType = mem_row.get(1);
                let member_type: ElementType = db_member_type.into();
                let member_id: i64 = mem_row.get(2);
                let member_role: String = mem_row.get(3);
                let member = RelationMember {
                    member_id,
                    member_type,
                    role: member_role,
                };
                if current_mem_id == relation.id {
                    relation.members.push(member);
                } else {
                    current_mem = Some(member);
                }
            }

            let el = Element::Relation(relation);
            callback(el)
        }

        Ok(())
    }
}
