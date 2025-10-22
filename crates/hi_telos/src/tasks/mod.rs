use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub id: Uuid,
    pub source: String,
    pub summary: String,
    pub telos_alignment: f32,
    pub created_at: DateTime<Utc>,
    #[serde(skip)]
    pub storage_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
pub struct IntentQueue {
    items: std::collections::VecDeque<Intent>,
}

impl IntentQueue {
    pub fn push(&mut self, intent: Intent) {
        self.items.push_back(intent);
    }

    pub fn push_front(&mut self, intent: Intent) {
        self.items.push_front(intent);
    }

    pub fn pop_next(&mut self) -> Option<Intent> {
        self.items.pop_front()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
