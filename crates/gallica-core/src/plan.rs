//! Cost estimates.

use gallica::QuotaClass;

use crate::settings::{Settings, VariantSpec};
use crate::state::{Estimate, PlanItem, Resolved};

/// Average number of views per issue, used until real page lists arrive.
pub const DEFAULT_VIEWS_PER_ISSUE: u32 = 8;

/// Rough size of one image for a variant.
pub fn bytes_guess_per_view(variant: VariantSpec) -> u64 {
    match variant {
        VariantSpec::Thumbnail => 8_000,
        VariantSpec::LowRes => 30_000,
        VariantSpec::MedRes => 90_000,
        VariantSpec::HighRes => 300_000,
        VariantSpec::Width(w) => {
            // Scale a 3 MB master (about 4300 px wide) by the area ratio.
            let ratio = (w as f64 / 4300.0).min(1.0);
            ((3_000_000.0 * ratio * ratio) as u64).max(20_000)
        }
        VariantSpec::Full => 3_000_000,
    }
}

/// Estimates the cost of downloading `items`.
pub fn estimate(resolved: &Resolved, items: &[PlanItem], settings: &Settings) -> Estimate {
    let views_known = items.iter().all(|i| i.views.is_some());
    let views: u64 = items
        .iter()
        .map(|i| i.views.unwrap_or(DEFAULT_VIEWS_PER_ISSUE) as u64)
        .sum();
    let image_class = settings.variant.quota_class();
    let quotas = settings.quotas();
    let text_requests = if settings.save_text {
        items.len() as u64
    } else {
        0
    };
    let mut minimum = quotas.minimum_duration(image_class, views);
    if text_requests > 0
        && let Some(t) = quotas.minimum_duration(QuotaClass::Text, text_requests)
    {
        minimum = Some(minimum.unwrap_or_default().max(t));
    }
    // Two metadata requests per item (record, page list), plus the per-year
    // issue lists already made while planning a periodical.
    let metadata_requests = 2 * items.len() as u64
        + if resolved.is_periodical() {
            items
                .iter()
                .filter_map(|i| i.year)
                .collect::<std::collections::BTreeSet<_>>()
                .len() as u64
        } else {
            0
        };
    Estimate {
        items: items.len(),
        views,
        views_known,
        metadata_requests,
        image_requests: views,
        image_class,
        text_requests,
        minimum,
        bytes_guess: views * bytes_guess_per_view(settings.variant),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gallica::Ark;

    fn items(n: usize, views: Option<u32>) -> Vec<PlanItem> {
        (0..n)
            .map(|i| PlanItem {
                id: i,
                ark: Ark::new(format!("bpt6k{i:08}")).unwrap(),
                date: None,
                year: Some(1900 + (i / 10) as u16),
                label: String::new(),
                views,
            })
            .collect()
    }

    #[test]
    fn estimates_use_quotas() {
        let resolved = Resolved::Periodical {
            ark: Ark::new("cb327110547").unwrap(),
            title: None,
            years: vec![],
            total_issues: None,
        };
        let mut settings = Settings {
            variant: VariantSpec::Full,
            ..Settings::default()
        };
        let e = estimate(&resolved, &items(20, None), &settings);
        assert_eq!(e.items, 20);
        assert_eq!(e.views, 160);
        assert!(!e.views_known);
        assert_eq!(e.image_class, QuotaClass::FullResImage);
        assert_eq!(e.minimum, Some(std::time::Duration::from_secs(60 * 31)));
        assert_eq!(e.metadata_requests, 40 + 2);
        assert_eq!(e.bytes_guess, 160 * 3_000_000);

        settings.variant = VariantSpec::Width(1000);
        let e = estimate(&resolved, &items(20, Some(10)), &settings);
        assert!(e.views_known);
        assert_eq!(e.views, 200);
        assert_eq!(e.image_class, QuotaClass::Default);
        assert_eq!(e.minimum, None);

        settings.save_text = true;
        let e = estimate(&resolved, &items(20, Some(10)), &settings);
        assert_eq!(e.text_requests, 20);
        assert_eq!(e.minimum, Some(std::time::Duration::from_secs(60 * 3)));
    }
}
