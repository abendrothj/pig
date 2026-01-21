//! Real-time metrics dashboard for workflow execution

use chrono::{DateTime, Local};
use std::collections::VecDeque;

/// Metrics collected during workflow execution
#[derive(Clone, Debug)]
pub struct ExecutionMetrics {
    pub workflow_start: Option<DateTime<Local>>,
    pub workflow_end: Option<DateTime<Local>>,
    pub step_metrics: Vec<StepMetric>,
    pub throughput_history: VecDeque<ThroughputSample>,
    pub memory_samples: VecDeque<MemorySample>,
    pub max_history_size: usize,
}

#[derive(Clone, Debug)]
pub struct StepMetric {
    pub step_id: String,
    pub step_name: String,
    pub start_time: DateTime<Local>,
    pub end_time: Option<DateTime<Local>>,
    pub duration_ms: Option<u64>,
    pub status: String, // "running" | "success" | "error" | "cached"
    pub tokens_processed: Option<u64>,
    pub memory_used_mb: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct ThroughputSample {
    pub timestamp: DateTime<Local>,
    pub tokens_per_sec: f64,
}

#[derive(Clone, Debug)]
pub struct MemorySample {
    pub timestamp: DateTime<Local>,
    pub memory_mb: f64,
}

impl Default for ExecutionMetrics {
    fn default() -> Self {
        Self {
            workflow_start: None,
            workflow_end: None,
            step_metrics: Vec::new(),
            throughput_history: VecDeque::new(),
            memory_samples: VecDeque::new(),
            max_history_size: 100,
        }
    }
}

impl ExecutionMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start tracking a workflow execution
    pub fn start_workflow(&mut self) {
        self.workflow_start = Some(Local::now());
        self.workflow_end = None;
        self.step_metrics.clear();
    }

    /// End workflow tracking
    pub fn end_workflow(&mut self) {
        self.workflow_end = Some(Local::now());
    }

    /// Add a step metric
    pub fn add_step(&mut self, step_id: String, step_name: String) {
        self.step_metrics.push(StepMetric {
            step_id,
            step_name,
            start_time: Local::now(),
            end_time: None,
            duration_ms: None,
            status: "running".to_string(),
            tokens_processed: None,
            memory_used_mb: None,
        });
    }

    /// Update step completion
    pub fn complete_step(&mut self, step_id: &str, status: &str, tokens: Option<u64>) {
        if let Some(metric) = self.step_metrics.iter_mut().find(|m| m.step_id == step_id) {
            metric.end_time = Some(Local::now());
            metric.status = status.to_string();
            metric.tokens_processed = tokens;
            
            if let Some(end) = metric.end_time {
                metric.duration_ms = Some(
                    (end - metric.start_time).num_milliseconds() as u64
                );
            }
        }
    }

    /// Add throughput sample
    pub fn add_throughput(&mut self, tokens_per_sec: f64) {
        self.throughput_history.push_back(ThroughputSample {
            timestamp: Local::now(),
            tokens_per_sec,
        });
        
        while self.throughput_history.len() > self.max_history_size {
            self.throughput_history.pop_front();
        }
    }

    /// Add memory sample
    pub fn add_memory_sample(&mut self, memory_mb: f64) {
        self.memory_samples.push_back(MemorySample {
            timestamp: Local::now(),
            memory_mb,
        });
        
        while self.memory_samples.len() > self.max_history_size {
            self.memory_samples.pop_front();
        }
    }

    /// Get workflow duration in seconds
    pub fn workflow_duration_secs(&self) -> Option<f64> {
        if let (Some(start), Some(end)) = (self.workflow_start, self.workflow_end) {
            Some((end - start).num_milliseconds() as f64 / 1000.0)
        } else if let Some(start) = self.workflow_start {
            // Still running
            Some((Local::now() - start).num_milliseconds() as f64 / 1000.0)
        } else {
            None
        }
    }

    /// Get average tokens per second
    pub fn avg_tokens_per_sec(&self) -> f64 {
        if self.throughput_history.is_empty() {
            return 0.0;
        }
        
        let sum: f64 = self.throughput_history.iter().map(|s| s.tokens_per_sec).sum();
        sum / self.throughput_history.len() as f64
    }

    /// Get peak tokens per second
    pub fn peak_tokens_per_sec(&self) -> f64 {
        self.throughput_history
            .iter()
            .map(|s| s.tokens_per_sec)
            .fold(0.0, f64::max)
    }

    /// Get current memory usage
    pub fn current_memory_mb(&self) -> f64 {
        self.memory_samples
            .back()
            .map(|s| s.memory_mb)
            .unwrap_or(0.0)
    }

    /// Get peak memory usage
    pub fn peak_memory_mb(&self) -> f64 {
        self.memory_samples
            .iter()
            .map(|s| s.memory_mb)
            .fold(0.0, f64::max)
    }

    /// Get total tokens processed
    pub fn total_tokens(&self) -> u64 {
        self.step_metrics
            .iter()
            .filter_map(|m| m.tokens_processed)
            .sum()
    }

    /// Get steps by status
    pub fn steps_by_status(&self, status: &str) -> usize {
        self.step_metrics
            .iter()
            .filter(|m| m.status == status)
            .count()
    }
}

/// Show metrics dashboard in UI
pub fn show_metrics_panel(ui: &mut egui::Ui, metrics: &ExecutionMetrics) {
    ui.group(|ui| {
        ui.heading("📊 Execution Metrics");
        ui.separator();

        // Workflow status
        ui.horizontal(|ui| {
            ui.label("Status:");
            if metrics.workflow_end.is_some() {
                ui.colored_label(egui::Color32::GREEN, "✓ Completed");
            } else if metrics.workflow_start.is_some() {
                ui.colored_label(egui::Color32::YELLOW, "⏳ Running");
            } else {
                ui.colored_label(egui::Color32::GRAY, "⏸ Idle");
            }
        });

        // Duration
        if let Some(duration) = metrics.workflow_duration_secs() {
            ui.horizontal(|ui| {
                ui.label("Duration:");
                ui.monospace(format!("{:.2}s", duration));
            });
        }

        ui.separator();

        // Step statistics
        ui.label(format!("Steps: {} total", metrics.step_metrics.len()));
        ui.horizontal(|ui| {
            let success = metrics.steps_by_status("success");
            let running = metrics.steps_by_status("running");
            let error = metrics.steps_by_status("error");
            let cached = metrics.steps_by_status("cached");
            
            if success > 0 {
                ui.colored_label(egui::Color32::GREEN, format!("✓ {}", success));
            }
            if running > 0 {
                ui.colored_label(egui::Color32::YELLOW, format!("⏳ {}", running));
            }
            if error > 0 {
                ui.colored_label(egui::Color32::RED, format!("✗ {}", error));
            }
            if cached > 0 {
                ui.colored_label(egui::Color32::from_rgb(156, 39, 176), format!("💾 {}", cached));
            }
        });

        ui.separator();

        // Performance metrics
        ui.label("Performance:");
        ui.horizontal(|ui| {
            ui.label("Avg:");
            ui.monospace(format!("{:.1} tok/s", metrics.avg_tokens_per_sec()));
        });
        ui.horizontal(|ui| {
            ui.label("Peak:");
            ui.monospace(format!("{:.1} tok/s", metrics.peak_tokens_per_sec()));
        });
        ui.horizontal(|ui| {
            ui.label("Tokens:");
            ui.monospace(format!("{}", metrics.total_tokens()));
        });

        ui.separator();

        // Memory usage
        ui.label("Memory:");
        ui.horizontal(|ui| {
            ui.label("Current:");
            ui.monospace(format!("{:.1} MB", metrics.current_memory_mb()));
        });
        ui.horizontal(|ui| {
            ui.label("Peak:");
            ui.monospace(format!("{:.1} MB", metrics.peak_memory_mb()));
        });

        // Throughput graph (simple)
        if !metrics.throughput_history.is_empty() {
            ui.separator();
            ui.label("Throughput History:");
            
            let history: Vec<f64> = metrics.throughput_history
                .iter()
                .map(|s| s.tokens_per_sec)
                .collect();
            
            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                let available = ui.available_size();
                let (response, painter) = ui.allocate_painter(
                    egui::vec2(available.x, 60.0),
                    egui::Sense::hover(),
                );
                
                let rect = response.rect;
                let max_value = history.iter().cloned().fold(1.0, f64::max);
                
                // Draw bars
                let bar_width = rect.width() / history.len() as f32;
                for (i, &value) in history.iter().enumerate() {
                    let x = rect.left() + (i as f32 * bar_width);
                    let height = (value / max_value) * rect.height() as f64;
                    let bar_rect = egui::Rect::from_min_size(
                        egui::pos2(x, rect.bottom() - height as f32),
                        egui::vec2(bar_width - 1.0, height as f32),
                    );
                    painter.rect_filled(bar_rect, 0.0, egui::Color32::from_rgb(33, 150, 243));
                }
            });
        }
    });
}
