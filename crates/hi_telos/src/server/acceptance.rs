use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::fs;

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct AcceptanceSummary {
    pub source: AcceptanceSource,
    pub metrics: AcceptanceMetrics,
    pub task_matrix: Vec<TaskMatrixEntry>,
    pub completed_todos: Vec<TodoItem>,
    pub pending_todos: Vec<TodoItem>,
    pub validation_plan: Vec<ValidationEntry>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct AcceptanceSource {
    pub doc_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct TodoItem {
    pub label: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct AcceptanceMetrics {
    pub modules_total: usize,
    pub modules_completed: usize,
    pub todos_completed: usize,
    pub todos_pending: usize,
    pub validation_steps: usize,
    pub overall_status: AcceptanceOverallStatus,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceOverallStatus {
    Complete,
    InProgress,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct TaskMatrixEntry {
    pub module: String,
    pub task: String,
    pub status: String,
}

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub struct ValidationEntry {
    pub kind: String,
    pub description: String,
    pub command: String,
}

pub async fn load_acceptance_summary(doc_path: &Path) -> anyhow::Result<AcceptanceSummary> {
    let metadata = fs::metadata(doc_path).await.ok();
    let updated_at = metadata
        .and_then(|meta| meta.modified().ok())
        .map(DateTime::<Utc>::from);

    let content = fs::read_to_string(doc_path)
        .await
        .with_context(|| format!("failed to read acceptance plan at {}", doc_path.display()))?;

    let ParsedAcceptancePlan {
        task_matrix,
        completed_todos,
        pending_todos,
        validation_plan,
    } = parse_acceptance_plan(&content);

    let metrics = AcceptanceMetrics {
        modules_total: task_matrix.len(),
        modules_completed: task_matrix
            .iter()
            .filter(|entry| status_indicates_completion(&entry.status))
            .count(),
        todos_completed: completed_todos.len(),
        todos_pending: pending_todos.len(),
        validation_steps: validation_plan.len(),
        overall_status: determine_overall_status(&task_matrix, &pending_todos),
    };

    Ok(AcceptanceSummary {
        source: AcceptanceSource {
            doc_path: doc_path.display().to_string(),
            updated_at,
        },
        metrics,
        task_matrix,
        completed_todos,
        pending_todos,
        validation_plan,
    })
}

struct ParsedAcceptancePlan {
    task_matrix: Vec<TaskMatrixEntry>,
    completed_todos: Vec<TodoItem>,
    pending_todos: Vec<TodoItem>,
    validation_plan: Vec<ValidationEntry>,
}

enum Section {
    None,
    TaskMatrix { header_rows_consumed: usize },
    Completed,
    Pending,
    ValidationTable { header_rows_consumed: usize },
}

fn parse_acceptance_plan(markdown: &str) -> ParsedAcceptancePlan {
    let mut section = Section::None;
    let mut task_matrix = Vec::new();
    let mut completed = Vec::new();
    let mut pending = Vec::new();
    let mut validation = Vec::new();

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## 2.") {
            section = Section::TaskMatrix {
                header_rows_consumed: 0,
            };
            continue;
        }
        if trimmed.starts_with("### 4.1") {
            section = Section::Completed;
            continue;
        }
        if trimmed.starts_with("### 4.2") {
            section = Section::Pending;
            continue;
        }
        if trimmed.starts_with("## 5.") {
            section = Section::ValidationTable {
                header_rows_consumed: 0,
            };
            continue;
        }
        if trimmed.starts_with("## ") && !trimmed.starts_with("## 5.") {
            section = Section::None;
            continue;
        }
        if trimmed.starts_with("### ") {
            section = Section::None;
            continue;
        }

        match &mut section {
            Section::TaskMatrix {
                header_rows_consumed,
            } => {
                if trimmed.starts_with('|') {
                    if *header_rows_consumed < 2 {
                        *header_rows_consumed += 1;
                        continue;
                    }

                    if let Some(entry) = parse_task_matrix_row(trimmed) {
                        task_matrix.push(entry);
                    }
                } else if !trimmed.is_empty() {
                    section = Section::None;
                }
            }
            Section::Completed => {
                if let Some(item) = parse_bullet(trimmed) {
                    completed.push(TodoItem { label: item });
                }
            }
            Section::Pending => {
                if let Some(item) = parse_bullet(trimmed) {
                    pending.push(TodoItem { label: item });
                }
            }
            Section::ValidationTable {
                header_rows_consumed,
            } => {
                if trimmed.starts_with('|') {
                    if *header_rows_consumed < 2 {
                        *header_rows_consumed += 1;
                        continue;
                    }

                    if let Some(entry) = parse_table_row(trimmed) {
                        validation.push(entry);
                    }
                } else if !trimmed.is_empty() {
                    section = Section::None;
                }
            }
            Section::None => {}
        }
    }

    ParsedAcceptancePlan {
        task_matrix,
        completed_todos: completed,
        pending_todos: pending,
        validation_plan: validation,
    }
}

fn parse_bullet(line: &str) -> Option<String> {
    if !line.starts_with('-') {
        return None;
    }

    let mut content = line.trim_start_matches('-').trim_start().to_string();
    if let Some(stripped) = content.strip_prefix("[x] ") {
        content = stripped.to_string();
    } else if let Some(stripped) = content.strip_prefix("[ ] ") {
        content = stripped.to_string();
    }

    if content.is_empty() {
        None
    } else {
        Some(content)
    }
}

fn parse_table_row(line: &str) -> Option<ValidationEntry> {
    let cells = parse_markdown_row_cells(line, 3)?;

    Some(ValidationEntry {
        kind: cells[0].clone(),
        description: cells[1].clone(),
        command: cells[2].clone(),
    })
}

fn parse_task_matrix_row(line: &str) -> Option<TaskMatrixEntry> {
    let cells = parse_markdown_row_cells(line, 3)?;

    Some(TaskMatrixEntry {
        module: cells[0].clone(),
        task: cells[1].clone(),
        status: cells[2].clone(),
    })
}

fn parse_markdown_row_cells(line: &str, expected_cells: usize) -> Option<Vec<String>> {
    if !line.starts_with('|') {
        return None;
    }

    let mut cells: Vec<_> = line.split('|').map(str::trim).collect();
    if cells.len() < expected_cells + 2 {
        return None;
    }

    if cells.first().map_or(false, |c| c.is_empty()) {
        cells.remove(0);
    }
    if cells.last().map_or(false, |c| c.is_empty()) {
        cells.pop();
    }

    if cells.len() != expected_cells {
        return None;
    }

    Some(cells.into_iter().map(|c| c.to_string()).collect())
}

fn determine_overall_status(
    task_matrix: &[TaskMatrixEntry],
    pending_todos: &[TodoItem],
) -> AcceptanceOverallStatus {
    let all_tasks_complete = task_matrix
        .iter()
        .all(|entry| status_indicates_completion(&entry.status));
    let no_pending = pending_todos.is_empty();

    if no_pending && (task_matrix.is_empty() || all_tasks_complete) {
        AcceptanceOverallStatus::Complete
    } else {
        AcceptanceOverallStatus::InProgress
    }
}

fn status_indicates_completion(status: &str) -> bool {
    let normalized = status.trim();
    if normalized.is_empty() {
        return false;
    }

    normalized.contains('✅')
        || normalized.chars().all(|c| c.is_whitespace() || c == '✅')
        || normalized.eq_ignore_ascii_case("done")
        || normalized.eq_ignore_ascii_case("complete")
        || normalized.contains("完成")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_acceptance_plan_extracts_sections() {
        let markdown = r#"
## 2. 任务矩阵
| 模块 | 任务 | 状态 |
| --- | --- | --- |
| API | Build endpoint | ✅ |

## 4. TODO 追踪

### 4.1 已完成清单
- [x] Completed item A
- [x] Completed item B

### 4.2 进行中/待定
- [ ] Pending item C
- 手动跟进事项

## 5. 验证方案概览
| 类型 | 验证内容 | 指令/方式 |
| --- | --- | --- |
| 端到端 | 测试闭环 | cargo test --test e2e |
| API | 校验接口 | curl http://localhost |

## 6. 其他
"#;

        let ParsedAcceptancePlan {
            task_matrix,
            completed_todos,
            pending_todos,
            validation_plan,
        } = parse_acceptance_plan(markdown);

        assert_eq!(
            task_matrix,
            vec![TaskMatrixEntry {
                module: "API".to_string(),
                task: "Build endpoint".to_string(),
                status: "✅".to_string(),
            }]
        );
        assert_eq!(
            completed_todos,
            vec![
                TodoItem {
                    label: "Completed item A".to_string(),
                },
                TodoItem {
                    label: "Completed item B".to_string(),
                }
            ]
        );
        assert_eq!(
            pending_todos,
            vec![
                TodoItem {
                    label: "Pending item C".to_string(),
                },
                TodoItem {
                    label: "手动跟进事项".to_string(),
                }
            ]
        );
        assert_eq!(
            validation_plan,
            vec![
                ValidationEntry {
                    kind: "端到端".to_string(),
                    description: "测试闭环".to_string(),
                    command: "cargo test --test e2e".to_string(),
                },
                ValidationEntry {
                    kind: "API".to_string(),
                    description: "校验接口".to_string(),
                    command: "curl http://localhost".to_string(),
                }
            ]
        );
    }

    #[tokio::test]
    async fn load_acceptance_summary_reads_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let doc_path = tmp.path().join("plan.md");
        tokio::fs::write(
            &doc_path,
            "## 4. TODO 追踪\n\n### 4.1 已完成清单\n- [x] Done\n\n### 4.2 进行中/待定\n- Pending\n\n## 5. 验证方案概览\n| 类型 | 验证内容 | 指令/方式 |\n| --- | --- | --- |\n| Demo | Validate | run demo |\n",
        )
        .await
        .expect("write plan");

        let summary = load_acceptance_summary(&doc_path)
            .await
            .expect("load summary");

        assert_eq!(summary.source.doc_path, doc_path.display().to_string());
        assert_eq!(summary.task_matrix.len(), 0);
        assert_eq!(summary.completed_todos.len(), 1);
        assert_eq!(summary.pending_todos.len(), 1);
        assert_eq!(summary.validation_plan.len(), 1);
        assert_eq!(summary.metrics.modules_total, 0);
        assert_eq!(summary.metrics.modules_completed, 0);
        assert_eq!(summary.metrics.todos_completed, 1);
        assert_eq!(summary.metrics.todos_pending, 1);
        assert_eq!(summary.metrics.validation_steps, 1);
        assert_eq!(
            summary.metrics.overall_status,
            AcceptanceOverallStatus::InProgress
        );
        assert!(summary.source.updated_at.is_some());
    }

    #[test]
    fn determine_overall_status_considers_modules_and_todos() {
        let task_matrix = vec![TaskMatrixEntry {
            module: "API".into(),
            task: "Expose endpoint".into(),
            status: "✅".into(),
        }];
        let pending = Vec::new();

        assert_eq!(
            determine_overall_status(&task_matrix, &pending),
            AcceptanceOverallStatus::Complete
        );

        let pending = vec![TodoItem {
            label: "Follow up".into(),
        }];

        assert_eq!(
            determine_overall_status(&task_matrix, &pending),
            AcceptanceOverallStatus::InProgress
        );
    }

    #[test]
    fn status_indicates_completion_handles_variants() {
        assert!(status_indicates_completion("✅"));
        assert!(status_indicates_completion(" 完成 ✅ "));
        assert!(status_indicates_completion("done"));
        assert!(status_indicates_completion("Complete"));
        assert!(status_indicates_completion("已完成"));
        assert!(!status_indicates_completion("进行中"));
        assert!(!status_indicates_completion(""));
    }
}
