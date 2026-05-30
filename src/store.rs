/// Metric data store: loads TensorBoard event files and organizes scalar metrics.
/// Supports both single-experiment and multi-experiment modes.
use crate::proto::{self, event, summary};
use crate::tfrecord::TfRecordReader;
use anyhow::Result;
use prost::Message;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct ScalarPoint {
    pub step: i64,
    pub value: f32,
    pub wall_time: f64,
}

#[derive(Debug, Clone)]
pub struct CachedFile {
    pub size: u64,
    pub modified: SystemTime,
    pub metrics: BTreeMap<String, Vec<ScalarPoint>>,
}

/// All scalar metrics from a single TensorBoard log directory.
#[derive(Debug, Clone)]
pub struct MetricStore {
    /// tag -> sorted list of points
    pub metrics: BTreeMap<String, Vec<ScalarPoint>>,
    pub event_files: Vec<PathBuf>,
    pub file_cache: BTreeMap<PathBuf, CachedFile>,
}

impl MetricStore {
    #[allow(dead_code)]
    pub fn load(dir: &Path) -> Result<Self> {
        let mut store = Self {
            metrics: BTreeMap::new(),
            event_files: Vec::new(),
            file_cache: BTreeMap::new(),
        };
        store.reload(dir)?;
        Ok(store)
    }

    pub fn reload(&mut self, dir: &Path) -> Result<()> {
        let event_files = find_event_files(dir)?;
        let mut new_file_cache = BTreeMap::new();
        let mut final_metrics: BTreeMap<String, Vec<ScalarPoint>> = BTreeMap::new();

        for path in &event_files {
            let Ok(meta) = std::fs::metadata(path) else {
                continue;
            };
            let size = meta.len();
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);

            // Check if we have a valid cache hit
            if let Some(cached) = self.file_cache.get(path) {
                if cached.size == size && cached.modified == modified {
                    // Cache hit! Reuse the parsed metrics
                    for (tag, points) in &cached.metrics {
                        final_metrics
                            .entry(tag.clone())
                            .or_default()
                            .extend(points.clone());
                    }
                    new_file_cache.insert(path.clone(), cached.clone());
                    continue;
                }
            }

            // Cache miss: parse the file
            let mut file_metrics: BTreeMap<String, Vec<ScalarPoint>> = BTreeMap::new();
            let Ok(file) = File::open(path) else {
                continue;
            };
            let reader = TfRecordReader::new(BufReader::new(file));

            for record in reader {
                let data = match record {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let event = match proto::Event::decode(data.as_slice()) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                if let Some(event::What::Summary(summary)) = event.what {
                    for val in summary.value {
                        if let Some(summary::value::Kind::SimpleValue(v)) = val.value {
                            file_metrics.entry(val.tag).or_default().push(ScalarPoint {
                                step: event.step,
                                value: v,
                                wall_time: event.wall_time,
                            });
                        }
                    }
                }
            }

            // Merge into final metrics
            for (tag, points) in &file_metrics {
                final_metrics
                    .entry(tag.clone())
                    .or_default()
                    .extend(points.clone());
            }

            // Save to new cache
            new_file_cache.insert(
                path.clone(),
                CachedFile {
                    size,
                    modified,
                    metrics: file_metrics,
                },
            );
        }

        // Sort points
        for points in final_metrics.values_mut() {
            points.sort_by_key(|p| p.step);
        }

        self.metrics = final_metrics;
        self.event_files = event_files;
        self.file_cache = new_file_cache;

        Ok(())
    }

    pub fn tags(&self) -> Vec<&str> {
        self.metrics.keys().map(|s| s.as_str()).collect()
    }
}

/// Multiple experiments, each with its own MetricStore.
#[derive(Debug, Clone)]
pub struct MultiStore {
    /// experiment name -> store
    pub experiments: Vec<(String, PathBuf, MetricStore)>,
}

impl MultiStore {
    /// Try to detect experiments in the given directory.
    /// Looks for subdirectories that contain a `tensorboard/` folder.
    /// Falls back to treating the directory itself as a single experiment.
    pub fn load(dir: &Path) -> Result<Self> {
        let mut store = Self { experiments: Vec::new() };
        store.reload(dir)?;
        Ok(store)
    }

    pub fn reload(&mut self, dir: &Path) -> Result<()> {
        let mut new_experiments = Vec::new();
        let mut subdirs_found = false;

        // Check if subdirs contain tensorboard/ folders
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut subdirs: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            subdirs.sort_by_key(|e| e.file_name());

            for entry in &subdirs {
                let tb_dir = entry.path().join("tensorboard");
                if tb_dir.is_dir() {
                    subdirs_found = true;
                    let name = entry.file_name().to_string_lossy().to_string();

                    // Find and take existing store to reuse its file cache
                    let mut existing_store = None;
                    for (n, p, _) in &self.experiments {
                        if n == &name && p == &tb_dir {
                            if let Some(pos) = self.experiments.iter().position(|(n, p, _)| n == &name && p == &tb_dir) {
                                let (_, _, s) = self.experiments.remove(pos);
                                existing_store = Some(s);
                                break;
                            }
                        }
                    }

                    let mut store = existing_store.unwrap_or_else(|| MetricStore {
                        metrics: BTreeMap::new(),
                        event_files: Vec::new(),
                        file_cache: BTreeMap::new(),
                    });

                    if store.reload(&tb_dir).is_ok() && !store.metrics.is_empty() {
                        new_experiments.push((name, tb_dir, store));
                    }
                }
            }
        }

        // Fallback: treat the directory itself as a single experiment
        if !subdirs_found {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "default".to_string());

            let mut existing_store = None;
            for (n, p, _) in &self.experiments {
                if n == &name && p == dir {
                    if let Some(pos) = self.experiments.iter().position(|(n, p, _)| n == &name && p == dir) {
                        let (_, _, s) = self.experiments.remove(pos);
                        existing_store = Some(s);
                        break;
                    }
                }
            }

            let mut store = existing_store.unwrap_or_else(|| MetricStore {
                metrics: BTreeMap::new(),
                event_files: Vec::new(),
                file_cache: BTreeMap::new(),
            });

            if store.reload(dir).is_ok() {
                new_experiments.push((name, dir.to_path_buf(), store));
            }
        }

        self.experiments = new_experiments;
        Ok(())
    }

    /// Union of all metric tags across all experiments.
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: BTreeMap<&str, ()> = BTreeMap::new();
        for (_, _, store) in &self.experiments {
            for tag in store.tags() {
                tags.entry(tag).or_default();
            }
        }
        tags.keys().map(|s| s.to_string()).collect()
    }

    pub fn experiment_names(&self) -> Vec<&str> {
        self.experiments.iter().map(|(n, _, _)| n.as_str()).collect()
    }

    pub fn total_event_files(&self) -> usize {
        self.experiments.iter().map(|(_, _, s)| s.event_files.len()).sum()
    }

    pub fn total_metrics(&self) -> usize {
        self.all_tags().len()
    }
}

fn find_event_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(dir).follow_links(true) {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy();
        if name.starts_with("events.out.tfevents.") {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}
