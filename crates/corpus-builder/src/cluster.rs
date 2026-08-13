//! Paraphrase clustering: group different miners' responses to the same
//! underlying (city, intent, time window) question.
//!
//! The feed never routes one question hash to two miners directly, but
//! the daemon issues templated paraphrases of the same question and
//! routes each paraphrase to a different miner. This module re-groups
//! those paraphrases after the fact, using only what is cheap and
//! auditable: the city named in the question text, the routed intent,
//! and whether the miner's own reported valid-time span overlaps another
//! response's span in the same bucket.

use std::collections::HashMap;

/// One response's contribution to clustering.
pub struct ClusterInput<'a> {
    pub row_id: &'a str,
    pub miner_slug: &'a str,
    pub city: &'static str,
    pub intent: &'a str,
    /// Inclusive start of this response's valid-time span, naive
    /// minute-precision UTC (lexicographically sortable).
    pub min_time: &'a str,
    /// Inclusive end of this response's valid-time span.
    pub max_time: &'a str,
}

/// A merged cluster: every response whose valid-time span overlaps a
/// chain of other responses in the same (city, intent) bucket. Includes
/// size-1 clusters (a response that did not overlap anything else in its
/// bucket); the caller filters those out when it wants only the
/// multi-miner clusters.
#[derive(Debug)]
pub struct Cluster {
    pub id: String,
    pub city: &'static str,
    pub intent: String,
    pub miner_slugs: Vec<String>,
}

impl Cluster {
    /// Number of distinct miners that contributed to this cluster.
    pub fn distinct_miner_count(&self) -> usize {
        self.miner_slugs.len()
    }
}

/// Shorten a `WEATHER_*` intent for use in a cluster id, e.g.
/// `"WEATHER_CHECK"` -> `"check"`.
fn short_intent(intent: &str) -> String {
    intent
        .strip_prefix("WEATHER_")
        .unwrap_or(intent)
        .to_lowercase()
}

/// Assign a cluster id to every input row.
///
/// Returns the cluster id for each `row_id`, plus the list of clusters
/// formed. Overlap is decided by treating each response's [min_time,
/// max_time] as an interval and merging intervals that intersect,
/// transitively, within each (city, intent) bucket.
pub fn build_clusters(inputs: &[ClusterInput]) -> (HashMap<String, String>, Vec<Cluster>) {
    let mut buckets: HashMap<(&str, &str), Vec<&ClusterInput>> = HashMap::new();
    for input in inputs {
        buckets
            .entry((input.city, input.intent))
            .or_default()
            .push(input);
    }

    let mut ordered_buckets: Vec<_> = buckets.into_iter().collect();
    ordered_buckets.sort_by(|a, b| a.0.cmp(&b.0));

    let mut assignment = HashMap::new();
    let mut clusters = Vec::new();

    for (key, mut items) in ordered_buckets {
        items.sort_by(|a, b| a.min_time.cmp(b.min_time));

        let mut group_idx = 0usize;
        let mut i = 0usize;
        while i < items.len() {
            let mut group_max = items[i].max_time;
            let mut j = i + 1;
            while j < items.len() && items[j].min_time <= group_max {
                if items[j].max_time > group_max {
                    group_max = items[j].max_time;
                }
                j += 1;
            }

            let group = &items[i..j];
            let id = format!("{}_{}_{}", key.0, short_intent(key.1), group_idx);

            let mut miner_slugs: Vec<String> =
                group.iter().map(|x| x.miner_slug.to_string()).collect();
            miner_slugs.sort();
            miner_slugs.dedup();

            for item in group {
                assignment.insert(item.row_id.to_string(), id.clone());
            }

            clusters.push(Cluster {
                id,
                city: key.0,
                intent: key.1.to_string(),
                miner_slugs,
            });

            group_idx += 1;
            i = j;
        }
    }

    (assignment, clusters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_spans_from_different_miners_merge_into_one_cluster() {
        let inputs = vec![
            ClusterInput {
                row_id: "a",
                miner_slug: "weatherapi",
                city: "dubai",
                intent: "WEATHER_CHECK",
                min_time: "2026-08-10T00:00",
                max_time: "2026-08-10T06:00",
            },
            ClusterInput {
                row_id: "b",
                miner_slug: "openweathermap",
                city: "dubai",
                intent: "WEATHER_CHECK",
                min_time: "2026-08-10T03:00",
                max_time: "2026-08-10T09:00",
            },
        ];
        let (assignment, clusters) = build_clusters(&inputs);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].distinct_miner_count(), 2);
        assert_eq!(assignment.get("a"), assignment.get("b"));
    }

    #[test]
    fn non_overlapping_spans_form_separate_clusters() {
        let inputs = vec![
            ClusterInput {
                row_id: "a",
                miner_slug: "weatherapi",
                city: "dubai",
                intent: "WEATHER_CHECK",
                min_time: "2026-08-10T00:00",
                max_time: "2026-08-10T01:00",
            },
            ClusterInput {
                row_id: "b",
                miner_slug: "openweathermap",
                city: "dubai",
                intent: "WEATHER_CHECK",
                min_time: "2026-08-12T00:00",
                max_time: "2026-08-12T01:00",
            },
        ];
        let (assignment, clusters) = build_clusters(&inputs);
        assert_eq!(clusters.len(), 2);
        assert_ne!(assignment.get("a"), assignment.get("b"));
    }

    #[test]
    fn different_intent_never_clusters_even_with_same_city_and_overlap() {
        let inputs = vec![
            ClusterInput {
                row_id: "a",
                miner_slug: "weatherapi",
                city: "dubai",
                intent: "WEATHER_CHECK",
                min_time: "2026-08-10T00:00",
                max_time: "2026-08-10T06:00",
            },
            ClusterInput {
                row_id: "b",
                miner_slug: "bittensor-sn18-zeus",
                city: "dubai",
                intent: "WEATHER_RISK_ASSESSMENT",
                min_time: "2026-08-10T00:00",
                max_time: "2026-08-10T06:00",
            },
        ];
        let (_assignment, clusters) = build_clusters(&inputs);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn transitive_overlap_chains_three_responses_into_one_cluster() {
        let inputs = vec![
            ClusterInput {
                row_id: "a",
                miner_slug: "weatherapi",
                city: "miami",
                intent: "WEATHER_FORECAST",
                min_time: "2026-08-10T00:00",
                max_time: "2026-08-10T05:00",
            },
            ClusterInput {
                row_id: "b",
                miner_slug: "openweathermap",
                city: "miami",
                intent: "WEATHER_FORECAST",
                min_time: "2026-08-10T04:00",
                max_time: "2026-08-10T10:00",
            },
            ClusterInput {
                row_id: "c",
                miner_slug: "bittensor-sn18-zeus",
                city: "miami",
                intent: "WEATHER_FORECAST",
                min_time: "2026-08-10T09:00",
                max_time: "2026-08-10T15:00",
            },
        ];
        let (_assignment, clusters) = build_clusters(&inputs);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].distinct_miner_count(), 3);
    }

    #[test]
    fn single_miner_forms_a_size_one_cluster() {
        let inputs = vec![ClusterInput {
            row_id: "a",
            miner_slug: "bittensor-sn18-zeus",
            city: "dubai",
            intent: "WEATHER_RISK_ASSESSMENT",
            min_time: "2026-08-10T00:00",
            max_time: "2026-08-10T06:00",
        }];
        let (_assignment, clusters) = build_clusters(&inputs);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].distinct_miner_count(), 1);
    }
}
