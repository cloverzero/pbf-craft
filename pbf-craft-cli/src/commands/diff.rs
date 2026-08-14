use std::fs::File;

use clap::Args;
use serde::{Deserialize, Serialize};

use pbf_craft::models::{Element, ElementType};
use pbf_craft::readers::IterableReader;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DiffType {
    Add,
    Modify,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementDiff {
    pub element_type: ElementType,
    pub element_id: i64,
    pub diff_type: DiffType,
}

#[derive(Args)]
pub struct DiffCommand {
    /// source pbf path
    #[clap(short, long, value_parser)]
    source: String,

    /// target pbf path
    #[clap(short, long, value_parser)]
    target: String,

    /// output path
    #[clap(short, long, value_parser, default_value = "./diff.csv")]
    output: String,
}

impl DiffCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let mut diff_csv = csv::WriterBuilder::new().from_writer(File::create(&self.output)?);

        let mut source = IterableReader::from_path(&self.source)?;
        let mut target = IterableReader::from_path(&self.target)?;

        let mut source_element_cnt = source.next();
        let mut target_element_cnt = target.next();

        macro_rules! record {
            ($element_type:expr, $element_id:expr, $diff_type:expr) => {
                diff_csv.serialize(ElementDiff {
                    element_type: $element_type,
                    element_id: $element_id,
                    diff_type: $diff_type,
                })?
            };
        }

        loop {
            match (&source_element_cnt, &target_element_cnt) {
                (Some(source_element), Some(target_element)) => {
                    match (source_element, target_element) {
                        (Element::Node(source_element), Element::Node(target_element)) => {
                            if source_element.id == target_element.id {
                                if source_element != target_element {
                                    record!(ElementType::Node, source_element.id, DiffType::Modify);
                                }
                                source_element_cnt = source.next();
                                target_element_cnt = target.next();
                            } else if source_element.id < target_element.id {
                                record!(ElementType::Node, source_element.id, DiffType::Delete);
                                source_element_cnt = source.next();
                            } else {
                                record!(ElementType::Node, target_element.id, DiffType::Add);
                                target_element_cnt = target.next();
                            }
                        }
                        (Element::Node(source_element), Element::Way(_)) => {
                            record!(ElementType::Node, source_element.id, DiffType::Delete);
                            source_element_cnt = source.next();
                        }
                        (Element::Way(_), Element::Node(target_element)) => {
                            record!(ElementType::Node, target_element.id, DiffType::Add);
                            target_element_cnt = target.next();
                        }
                        (Element::Way(source_element), Element::Way(target_element)) => {
                            if source_element.id == target_element.id {
                                if source_element != target_element {
                                    record!(ElementType::Way, source_element.id, DiffType::Modify);
                                }
                                source_element_cnt = source.next();
                                target_element_cnt = target.next();
                            } else if source_element.id < target_element.id {
                                record!(ElementType::Way, source_element.id, DiffType::Delete);
                                source_element_cnt = source.next();
                            } else {
                                record!(ElementType::Way, target_element.id, DiffType::Add);
                                target_element_cnt = target.next();
                            }
                        }
                        (Element::Way(source_way), Element::Relation(_)) => {
                            record!(ElementType::Way, source_way.id, DiffType::Delete);
                            source_element_cnt = source.next();
                        }
                        (Element::Relation(_), Element::Way(target_way)) => {
                            record!(ElementType::Way, target_way.id, DiffType::Add);
                            target_element_cnt = target.next();
                        }
                        (Element::Relation(source_element), Element::Relation(target_element)) => {
                            if source_element.id == target_element.id {
                                if source_element != target_element {
                                    record!(
                                        ElementType::Relation,
                                        source_element.id,
                                        DiffType::Modify
                                    );
                                }
                                source_element_cnt = source.next();
                                target_element_cnt = target.next();
                            } else if source_element.id < target_element.id {
                                record!(ElementType::Relation, source_element.id, DiffType::Delete);
                                source_element_cnt = source.next();
                            } else {
                                record!(ElementType::Relation, target_element.id, DiffType::Add);
                                target_element_cnt = target.next();
                            }
                        }
                        (Element::Relation(_), Element::Node(target_node)) => {
                            record!(ElementType::Node, target_node.id, DiffType::Add);
                            target_element_cnt = target.next();
                        }
                        (Element::Node(source_node), Element::Relation(_)) => {
                            record!(ElementType::Node, source_node.id, DiffType::Delete);
                            source_element_cnt = source.next();
                        }
                    }
                }
                (Some(source_element), None) => {
                    let (element_type, element_id) = source_element.get_meta();
                    record!(element_type, element_id, DiffType::Delete);
                    source_element_cnt = source.next();
                }
                (None, Some(target_element)) => {
                    let (element_type, element_id) = target_element.get_meta();
                    record!(element_type, element_id, DiffType::Add);
                    target_element_cnt = target.next();
                }
                (None, None) => break,
            }
        }

        diff_csv.flush()?;
        println!("Diff file created: ./{}", &self.output);
        Ok(())
    }
}
