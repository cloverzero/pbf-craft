use std::fs::File;
use std::io::BufReader;
use std::time::{Duration, Instant};

use clap::Args;
use serde::{Deserialize, Serialize};

use pbf_craft::models::{Element, ElementType};
use pbf_craft::readers::{IterableReader, ReaderProgress};

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

/// Element type rank for (type, id) ordering: node < way < relation.
fn element_rank(element_type: &ElementType) -> u8 {
    match element_type {
        ElementType::Node => 0,
        ElementType::Way => 1,
        ElementType::Relation => 2,
    }
}

/// Wraps an `IterableReader` and verifies the stream is ordered by (type, id) — the invariant
/// the merge-based diff relies on. A violation (unordered file) is reported as an error
/// instead of silently producing wrong diff results.
struct SortedElementStream {
    iter: IterableReader<BufReader<File>>,
    prev: Option<(u8, i64)>,
}

impl SortedElementStream {
    fn new(path: &str) -> anyhow::Result<Self> {
        Ok(Self {
            iter: IterableReader::from_path(path)?,
            prev: None,
        })
    }

    /// Passes through the underlying reader's progress (percent of each file consumed).
    fn progress(&self) -> ReaderProgress {
        self.iter.progress()
    }

    fn next(&mut self) -> anyhow::Result<Option<Element>> {
        match self.iter.next() {
            Some(element) => {
                let (element_type, element_id) = element.get_meta();
                let key = (element_rank(&element_type), element_id);
                if let Some(prev) = self.prev {
                    if key < prev {
                        bail!(
                            "input stream is not ordered by (type, id): {:?} follows {:?}; \
                             the diff command requires sorted files (all nodes by id, then all \
                             ways, then all relations)",
                            key,
                            prev
                        );
                    }
                }
                self.prev = Some(key);
                Ok(Some(element))
            }
            None => Ok(None),
        }
    }
}

impl DiffCommand {
    pub fn run(self) -> anyhow::Result<()> {
        let mut diff_csv = csv::WriterBuilder::new().from_writer(File::create(&self.output)?);

        let mut source = SortedElementStream::new(&self.source)?;
        let mut target = SortedElementStream::new(&self.target)?;

        let mut source_element_cnt = source.next()?;
        let mut target_element_cnt = target.next()?;

        // Progress reporting: poll every ~4096 iterations, but at most every 250 ms, and
        // render an overwritten line on stderr so the CSV output stays clean.
        let mut last_report = Instant::now();
        let mut iterations = 0u64;
        macro_rules! report_progress {
            () => {
                if iterations % 4096 == 0 && last_report.elapsed() >= Duration::from_millis(250) {
                    let progress_text = |progress: ReaderProgress| match progress.percent() {
                        Some(p) => format!("{:.1}%", p),
                        None => format!("{} bytes", progress.bytes_read),
                    };
                    eprint!(
                        "\rdiff: source {}, target {}   ",
                        progress_text(source.progress()),
                        progress_text(target.progress()),
                    );
                    last_report = Instant::now();
                }
            };
        }

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
            iterations += 1;
            report_progress!();
            match (&source_element_cnt, &target_element_cnt) {
                (Some(source_element), Some(target_element)) => {
                    match (source_element, target_element) {
                        (Element::Node(source_element), Element::Node(target_element)) => {
                            if source_element.id == target_element.id {
                                if source_element != target_element {
                                    record!(ElementType::Node, source_element.id, DiffType::Modify);
                                }
                                source_element_cnt = source.next()?;
                                target_element_cnt = target.next()?;
                            } else if source_element.id < target_element.id {
                                record!(ElementType::Node, source_element.id, DiffType::Delete);
                                source_element_cnt = source.next()?;
                            } else {
                                record!(ElementType::Node, target_element.id, DiffType::Add);
                                target_element_cnt = target.next()?;
                            }
                        }
                        (Element::Node(source_element), Element::Way(_)) => {
                            record!(ElementType::Node, source_element.id, DiffType::Delete);
                            source_element_cnt = source.next()?;
                        }
                        (Element::Way(_), Element::Node(target_element)) => {
                            record!(ElementType::Node, target_element.id, DiffType::Add);
                            target_element_cnt = target.next()?;
                        }
                        (Element::Way(source_element), Element::Way(target_element)) => {
                            if source_element.id == target_element.id {
                                if source_element != target_element {
                                    record!(ElementType::Way, source_element.id, DiffType::Modify);
                                }
                                source_element_cnt = source.next()?;
                                target_element_cnt = target.next()?;
                            } else if source_element.id < target_element.id {
                                record!(ElementType::Way, source_element.id, DiffType::Delete);
                                source_element_cnt = source.next()?;
                            } else {
                                record!(ElementType::Way, target_element.id, DiffType::Add);
                                target_element_cnt = target.next()?;
                            }
                        }
                        (Element::Way(source_way), Element::Relation(_)) => {
                            record!(ElementType::Way, source_way.id, DiffType::Delete);
                            source_element_cnt = source.next()?;
                        }
                        (Element::Relation(_), Element::Way(target_way)) => {
                            record!(ElementType::Way, target_way.id, DiffType::Add);
                            target_element_cnt = target.next()?;
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
                                source_element_cnt = source.next()?;
                                target_element_cnt = target.next()?;
                            } else if source_element.id < target_element.id {
                                record!(ElementType::Relation, source_element.id, DiffType::Delete);
                                source_element_cnt = source.next()?;
                            } else {
                                record!(ElementType::Relation, target_element.id, DiffType::Add);
                                target_element_cnt = target.next()?;
                            }
                        }
                        (Element::Relation(_), Element::Node(target_node)) => {
                            record!(ElementType::Node, target_node.id, DiffType::Add);
                            target_element_cnt = target.next()?;
                        }
                        (Element::Node(source_node), Element::Relation(_)) => {
                            record!(ElementType::Node, source_node.id, DiffType::Delete);
                            source_element_cnt = source.next()?;
                        }
                    }
                }
                (Some(source_element), None) => {
                    let (element_type, element_id) = source_element.get_meta();
                    record!(element_type, element_id, DiffType::Delete);
                    source_element_cnt = source.next()?;
                }
                (None, Some(target_element)) => {
                    let (element_type, element_id) = target_element.get_meta();
                    record!(element_type, element_id, DiffType::Add);
                    target_element_cnt = target.next()?;
                }
                (None, None) => break,
            }
        }

        // Terminate the progress line.
        eprintln!();

        diff_csv.flush()?;
        println!("Diff file created: ./{}", &self.output);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pbf_craft::models::Node;
    use pbf_craft::writers::PbfWriter;

    #[test]
    fn sorted_stream_rejects_unordered_input() {
        let path =
            std::env::temp_dir().join(format!("pbf_craft_diff_test_{}.pbf", std::process::id()));
        // Node ids 2, 1 (out of order) yield an element stream that is not sorted by id.
        let mut writer = PbfWriter::from_path(&path, true).unwrap();
        writer
            .write(pbf_craft::models::Element::Node(Node {
                id: 2,
                ..Default::default()
            }))
            .unwrap();
        writer
            .write(pbf_craft::models::Element::Node(Node {
                id: 1,
                ..Default::default()
            }))
            .unwrap();
        writer.finish().unwrap();

        let mut stream = SortedElementStream::new(path.to_str().unwrap()).unwrap();
        assert!(stream.next().unwrap().is_some()); // node 2
        let err = stream.next().unwrap_err(); // node 1 follows node 2 -> invariant violation
        assert!(
            err.to_string().contains("not ordered"),
            "expected an ordering error, got: {}",
            err
        );
        let _ = std::fs::remove_file(&path);
    }
}
