use std::path;

pub(crate) fn exists(filepath: &str) -> bool {
    let file = path::Path::new(filepath);
    file.exists()
}
