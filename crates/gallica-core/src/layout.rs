//! Where downloaded files go.
//!
//! ```text
//! <output_dir>/
//!   cb327110547 - Les Belles images/        periodical: "<ark> - <title>"
//!     periodical.json
//!     job.json
//!     1921/
//!       1921-11-03 bpt6k961263c/            "<date> <ark>"
//!         metadata.json
//!         f0001.jpg …
//!         text.html                          only with save_text
//!         done.json
//!   bpt6k961287k - Les Belles images/       single document: same inner layout
//! ```

use std::path::{Path, PathBuf};

use crate::state::{PlanItem, Resolved};

/// Per-item bibliographic record and page list.
pub const METADATA_FILE: &str = "metadata.json";
/// Per-item completion marker.
pub const DONE_FILE: &str = "done.json";
/// Per-item plain text export.
pub const TEXT_FILE: &str = "text.html";
/// Per-target description of the last run.
pub const JOB_FILE: &str = "job.json";
/// Per-periodical description.
pub const PERIODICAL_FILE: &str = "periodical.json";

/// Maximum length of a sanitised title in a folder name.
pub const MAX_TITLE_LEN: usize = 80;

/// Makes a title safe for every common file system: forbidden characters
/// become spaces, whitespace is collapsed, trailing dots and spaces are
/// removed and the result is cut at [`MAX_TITLE_LEN`] characters.
pub fn sanitize(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(MAX_TITLE_LEN).collect();
    while out.ends_with(['.', ' ']) {
        out.pop();
    }
    out
}

/// The folder of a resolved target inside the output directory.
pub fn target_dir(output_dir: &Path, resolved: &Resolved) -> PathBuf {
    let ark = resolved.ark().name();
    let name = match resolved.title().map(sanitize).filter(|t| !t.is_empty()) {
        Some(title) => format!("{ark} - {title}"),
        None => ark.to_string(),
    };
    output_dir.join(name)
}

/// The folder of one plan item. For a periodical, issues are grouped by year;
/// for a single document, the target folder itself.
pub fn item_dir(target_dir: &Path, resolved: &Resolved, item: &PlanItem) -> PathBuf {
    if !resolved.is_periodical() {
        return target_dir.to_path_buf();
    }
    let ark = item.ark.name();
    match (item.date, item.year) {
        (Some(date), _) => target_dir
            .join(date.year.to_string())
            .join(format!("{date} {ark}")),
        (None, Some(year)) => target_dir
            .join(year.to_string())
            .join(format!("{year} {ark}")),
        (None, None) => target_dir.join(ark),
    }
}

/// The file name of a view image inside an item folder.
pub fn view_file_name(variant: &gallica::ImageVariant, view: u32) -> String {
    variant.file_name(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gallica::Ark;
    use gallica::document::Date;

    #[test]
    fn sanitises_titles() {
        assert_eq!(
            sanitize("Le Figaro : journal / dir. H. de Villemessant..."),
            "Le Figaro journal dir. H. de Villemessant"
        );
        assert_eq!(sanitize("  a  b\tc  "), "a b c");
        assert_eq!(sanitize(&"x".repeat(200)).len(), MAX_TITLE_LEN);
        assert_eq!(sanitize("???"), "");
    }

    #[test]
    fn builds_paths() {
        let resolved = Resolved::Periodical {
            ark: Ark::new("cb327110547").unwrap(),
            title: Some("Les Belles images".into()),
            years: vec![1921],
            total_issues: None,
        };
        let target = target_dir(Path::new("/out"), &resolved);
        assert_eq!(target, Path::new("/out/cb327110547 - Les Belles images"));
        let item = PlanItem {
            id: 0,
            ark: Ark::new("bpt6k961263c").unwrap(),
            date: Date::from_day_of_year(1921, 307),
            year: Some(1921),
            label: "03 novembre 1921".into(),
            views: None,
        };
        assert_eq!(
            item_dir(&target, &resolved, &item),
            Path::new("/out/cb327110547 - Les Belles images/1921/1921-11-03 bpt6k961263c")
        );
        let undated = PlanItem {
            date: None,
            ..item.clone()
        };
        assert_eq!(
            item_dir(&target, &resolved, &undated),
            Path::new("/out/cb327110547 - Les Belles images/1921/1921 bpt6k961263c")
        );

        let doc = Resolved::Document {
            ark: Ark::new("bpt6k961287k").unwrap(),
            title: None,
            date: None,
            typedoc: None,
            views: 8,
            rights: vec![],
        };
        let target = target_dir(Path::new("/out"), &doc);
        assert_eq!(target, Path::new("/out/bpt6k961287k"));
        assert_eq!(item_dir(&target, &doc, &item), target);
    }
}
