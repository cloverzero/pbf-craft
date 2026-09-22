use std::str::FromStr;

use clap::Args;
use colored_json::prelude::*;

use pbf_craft::models::{Element, ElementType, Tag};
use pbf_craft::readers::{IndexedReader, PbfReader};

#[derive(Args, Debug)]
pub struct SearchCommand {
    /// element type: node, way, relation
    #[clap(long, value_parser)]
    eltype: Option<String>,

    /// element id
    #[clap(long, value_parser)]
    elid: Option<i64>,

    /// tag key (substring match against tag keys)
    #[clap(long, value_parser)]
    tagkey: Option<String>,

    /// tag value (substring match against tag values)
    #[clap(long, value_parser)]
    tagvalue: Option<String>,

    #[clap(long, value_parser)]
    pair: Option<Vec<i64>>,

    /// file path
    #[clap(short, long, value_parser)]
    file: String,

    /// The default value is true. If true, it will match exactly the only element. If false, all associated elements will be matched.
    #[clap(short, long, value_parser)]
    exact: Option<bool>,
}

impl SearchCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let result = if let (Some(eltype), Some(elid)) = (&self.eltype, &self.elid) {
            blue!("Searching ");
            dark_yellow!("{} ", &self.file);
            blue!("for ");
            dark_yellow!("{}#{} ", eltype, elid);
            println!("...");

            let element_type = ElementType::from_str(eltype)?;

            if self.exact.is_none() || self.exact.unwrap() {
                let mut indexed_reader = IndexedReader::from_path(&self.file)?;
                let find_result = indexed_reader.find(&element_type, *elid)?;
                match find_result {
                    Some(ec) => {
                        vec![ec]
                    }
                    None => Vec::with_capacity(0),
                }
            } else {
                let reader = PbfReader::from_path(&self.file)?;
                reader.par_find(None, |element| match (element, &element_type) {
                    (Element::Node(node), ElementType::Node) => node.id == *elid,
                    (Element::Way(way), ElementType::Node) => {
                        for way_node in &way.way_nodes {
                            if way_node.id == *elid {
                                return true;
                            }
                        }

                        false
                    }
                    (Element::Way(way), ElementType::Way) => way.id == *elid,
                    (Element::Relation(relation), ElementType::Relation) => relation.id == *elid,
                    (Element::Relation(relation), _) => {
                        for member in &relation.members {
                            if member.member_id == *elid && member.member_type.eq(&element_type) {
                                return true;
                            }
                        }

                        false
                    }
                    _ => false,
                })?
            }
        } else if self.tagkey.is_some() || self.tagvalue.is_some() {
            blue!("Searching ");
            dark_yellow!("{} ", &self.file);
            blue!("for ");
            dark_yellow!(
                "elements of tag key {:?} and tag value {:?} ",
                &self.tagkey,
                &self.tagvalue
            );
            println!("...");
            let reader = PbfReader::from_path(&self.file)?;
            reader.par_find(None, |element| match element {
                Element::Node(node) => does_tag_match(&node.tags, &self.tagkey, &self.tagvalue),
                Element::Way(way) => does_tag_match(&way.tags, &self.tagkey, &self.tagvalue),
                Element::Relation(relation) => {
                    does_tag_match(&relation.tags, &self.tagkey, &self.tagvalue)
                }
            })?
        } else if self.pair.is_some() {
            let node_ids = self.pair.unwrap();
            if node_ids.len() < 2 {
                anyhow::bail!("At least two nodes are required");
            }
            let first = node_ids[0];
            let second = node_ids[1];
            blue!("Searching ");
            dark_yellow!("{} ", &self.file);
            blue!("for ");
            dark_yellow!("ways containing the node pair of {} and {} ", first, second);
            println!("...");
            let reader = PbfReader::from_path(&self.file)?;
            reader.par_find(Some(&ElementType::Way), |el| {
                if let Element::Way(way) = el {
                    return way.way_nodes.iter().any(|ref_node| ref_node.id == first)
                        && way.way_nodes.iter().any(|ref_node| ref_node.id == second);
                }

                false
            })?
        } else {
            yellow!("Your input is incorrect");
            Vec::with_capacity(0)
        };

        println!(
            "{}",
            serde_json::to_string_pretty(&result)?.to_colored_json_auto()?
        );
        println!("{} elements found", result.len());
        Ok(())
    }
}

fn does_tag_match(tags: &Vec<Tag>, key: &Option<String>, value: &Option<String>) -> bool {
    for tag in tags {
        match (key, value) {
            (Some(k), Some(v)) => {
                if tag.key.contains(k) && tag.value.contains(v) {
                    return true;
                }
            }
            (Some(k), None) => {
                if tag.key.contains(k) {
                    return true;
                }
            }
            (None, Some(v)) => {
                if tag.value.contains(v) {
                    return true;
                }
            }
            (None, None) => return true,
        }
    }
    false
}
