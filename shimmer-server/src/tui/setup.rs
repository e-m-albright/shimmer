//! Interactive TUI setup wizard for initial server configuration.
//!
//! Guides the operator through storage backend selection, database path,
//! org creation, admin account creation, and optional SMTP configuration.
//! Writes the resulting config to `shimmer.toml` with 0600 permissions.

use std::io;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};

// ---------------------------------------------------------------------------
// Config output struct
// ---------------------------------------------------------------------------

/// Collects all values gathered by the setup wizard.
#[derive(Debug, Default)]
pub struct SetupConfig {
    pub bind: String,
    pub storage_backend: String,
    pub storage_path: String,
    pub s3_endpoint: String,
    pub s3_bucket: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub db_path: String,
    pub org_name: String,
    pub admin_email: String,
    pub admin_password: String,
    pub smtp_host: String,
    pub smtp_port: String,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_from: String,
    pub skip_smtp: bool,
}

// ---------------------------------------------------------------------------
// Wizard steps
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Step {
    StorageBackend,
    StoragePath,
    S3Config,
    DbPath,
    OrgName,
    AdminEmail,
    AdminPassword,
    SmtpChoice,
    SmtpConfig,
    Confirm,
    Done,
}

impl Step {
    fn title(&self) -> &'static str {
        match self {
            Self::StorageBackend => "Storage Backend",
            Self::StoragePath => "Storage Path",
            Self::S3Config => "S3 Configuration",
            Self::DbPath => "Database Path",
            Self::OrgName => "Organisation Name",
            Self::AdminEmail => "Admin Email",
            Self::AdminPassword => "Admin Password",
            Self::SmtpChoice => "Email (SMTP)",
            Self::SmtpConfig => "SMTP Configuration",
            Self::Confirm => "Confirm",
            Self::Done => "Done",
        }
    }

    fn index(&self) -> usize {
        match self {
            Self::StorageBackend => 0,
            Self::StoragePath => 1,
            Self::S3Config => 2,
            Self::DbPath => 3,
            Self::OrgName => 4,
            Self::AdminEmail => 5,
            Self::AdminPassword => 6,
            Self::SmtpChoice => 7,
            Self::SmtpConfig => 8,
            Self::Confirm => 9,
            Self::Done => 10,
        }
    }

    /// Total number of user-visible steps (not counting Done).
    fn total() -> usize {
        10
    }
}

// ---------------------------------------------------------------------------
// Wizard state
// ---------------------------------------------------------------------------

struct Wizard {
    step: Step,
    config: SetupConfig,
    /// Current text input buffer.
    input: String,
    /// Selection list state (for backend choice, smtp choice).
    list_state: ListState,
    /// Error message to display on current step.
    error: Option<String>,
    /// Whether the wizard was cancelled by the user.
    cancelled: bool,
}

impl Wizard {
    fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            step: Step::StorageBackend,
            config: SetupConfig {
                bind: "0.0.0.0:8443".into(),
                storage_backend: "file".into(),
                storage_path: "./shimmer-storage".into(),
                db_path: "./shimmer-metadata.db".into(),
                smtp_port: "587".into(),
                skip_smtp: true,
                ..SetupConfig::default()
            },
            input: String::new(),
            list_state,
            error: None,
            cancelled: false,
        }
    }

    // -----------------------------------------------------------------------
    // Navigation helpers
    // -----------------------------------------------------------------------

    fn advance(&mut self) {
        self.error = None;
        match &self.step {
            Step::StorageBackend => {
                let idx = self.list_state.selected().unwrap_or(0);
                self.config.storage_backend = if idx == 0 { "file".into() } else { "s3".into() };
                self.input = self.config.storage_path.clone();
                if self.config.storage_backend == "s3" {
                    self.step = Step::S3Config;
                    self.input = self.config.s3_endpoint.clone();
                } else {
                    self.step = Step::StoragePath;
                }
            }
            Step::StoragePath => {
                let val = self.input.trim().to_string();
                if val.is_empty() {
                    self.error = Some("Storage path cannot be empty.".into());
                    return;
                }
                self.config.storage_path = val;
                self.input = self.config.db_path.clone();
                self.step = Step::DbPath;
            }
            Step::S3Config => {
                // S3Config is handled with sub-fields. We store all in the input
                // buffer as "endpoint|bucket|access_key|secret_key".
                let parts: Vec<&str> = self.input.splitn(4, '|').collect();
                self.config.s3_endpoint = parts.first().unwrap_or(&"").to_string();
                self.config.s3_bucket = parts
                    .get(1)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&"shimmer")
                    .to_string();
                self.config.s3_access_key = parts.get(2).unwrap_or(&"").to_string();
                self.config.s3_secret_key = parts.get(3).unwrap_or(&"").to_string();
                if self.config.s3_bucket.is_empty() {
                    self.error = Some("S3 bucket name cannot be empty.".into());
                    return;
                }
                self.input = self.config.db_path.clone();
                self.step = Step::DbPath;
            }
            Step::DbPath => {
                let val = self.input.trim().to_string();
                if val.is_empty() {
                    self.error = Some("Database path cannot be empty.".into());
                    return;
                }
                self.config.db_path = val;
                self.input = self.config.org_name.clone();
                self.step = Step::OrgName;
            }
            Step::OrgName => {
                let val = self.input.trim().to_string();
                if val.is_empty() {
                    self.error = Some("Organisation name cannot be empty.".into());
                    return;
                }
                self.config.org_name = val;
                self.input = self.config.admin_email.clone();
                self.step = Step::AdminEmail;
            }
            Step::AdminEmail => {
                let val = self.input.trim().to_string();
                if !val.contains('@') {
                    self.error = Some("Enter a valid email address.".into());
                    return;
                }
                self.config.admin_email = val;
                self.input = String::new();
                self.step = Step::AdminPassword;
            }
            Step::AdminPassword => {
                let val = self.input.clone();
                if val.len() < 8 {
                    self.error = Some("Password must be at least 8 characters.".into());
                    return;
                }
                self.config.admin_password = val;
                self.input = String::new();
                self.list_state.select(Some(1)); // default: skip SMTP
                self.step = Step::SmtpChoice;
            }
            Step::SmtpChoice => {
                let idx = self.list_state.selected().unwrap_or(1);
                self.config.skip_smtp = idx == 1;
                if self.config.skip_smtp {
                    self.step = Step::Confirm;
                } else {
                    self.input = self.config.smtp_host.clone();
                    self.step = Step::SmtpConfig;
                }
            }
            Step::SmtpConfig => {
                // Format: "host|port|username|password|from"
                let parts: Vec<&str> = self.input.splitn(5, '|').collect();
                self.config.smtp_host = parts.first().unwrap_or(&"").to_string();
                self.config.smtp_port = parts
                    .get(1)
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&"587")
                    .to_string();
                self.config.smtp_username = parts.get(2).unwrap_or(&"").to_string();
                self.config.smtp_password = parts.get(3).unwrap_or(&"").to_string();
                self.config.smtp_from = parts.get(4).unwrap_or(&"").to_string();
                if self.config.smtp_host.is_empty() {
                    self.error = Some("SMTP host cannot be empty.".into());
                    return;
                }
                self.step = Step::Confirm;
            }
            Step::Confirm => {
                self.step = Step::Done;
            }
            Step::Done => {}
        }
    }

    fn go_back(&mut self) {
        self.error = None;
        match &self.step {
            Step::StoragePath | Step::S3Config => {
                self.step = Step::StorageBackend;
                self.list_state
                    .select(Some(if self.config.storage_backend == "s3" {
                        1
                    } else {
                        0
                    }));
            }
            Step::DbPath => {
                if self.config.storage_backend == "s3" {
                    self.step = Step::S3Config;
                    self.input = format!(
                        "{}|{}|{}|{}",
                        self.config.s3_endpoint,
                        self.config.s3_bucket,
                        self.config.s3_access_key,
                        self.config.s3_secret_key
                    );
                } else {
                    self.step = Step::StoragePath;
                    self.input = self.config.storage_path.clone();
                }
            }
            Step::OrgName => {
                self.step = Step::DbPath;
                self.input = self.config.db_path.clone();
            }
            Step::AdminEmail => {
                self.step = Step::OrgName;
                self.input = self.config.org_name.clone();
            }
            Step::AdminPassword => {
                self.step = Step::AdminEmail;
                self.input = self.config.admin_email.clone();
            }
            Step::SmtpChoice => {
                self.step = Step::AdminPassword;
                self.input = String::new();
            }
            Step::SmtpConfig => {
                self.step = Step::SmtpChoice;
                self.list_state.select(Some(0));
            }
            Step::Confirm => {
                if self.config.skip_smtp {
                    self.step = Step::SmtpChoice;
                    self.list_state.select(Some(1));
                } else {
                    self.step = Step::SmtpConfig;
                }
            }
            Step::StorageBackend | Step::Done => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(f: &mut ratatui::Frame, wizard: &Wizard) {
    let area = f.area();

    // Outer block
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " Shimmer Server Setup ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    // Layout: progress bar | main content | hint bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(inner);

    // Progress bar
    let progress_idx = wizard.step.index();
    let total = Step::total();
    let ratio = if wizard.step == Step::Done {
        1.0_f64
    } else {
        progress_idx as f64 / total as f64
    };
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::BOTTOM))
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
        .ratio(ratio)
        .label(format!("Step {} / {}", progress_idx + 1, total));
    f.render_widget(gauge, chunks[0]);

    // Main content area
    render_step(f, wizard, chunks[1]);

    // Hint bar
    let hints = match &wizard.step {
        Step::StorageBackend | Step::SmtpChoice => "↑/↓  navigate  │  Enter  confirm  │  Esc  quit",
        Step::Confirm => "Enter  confirm & write config  │  ←  back  │  Esc  quit",
        Step::Done => "Press any key to exit",
        _ => "Enter  next  │  ←  back  │  Esc  quit",
    };
    let hint_paragraph = Paragraph::new(hints)
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    f.render_widget(hint_paragraph, chunks[2]);
}

fn render_step(f: &mut ratatui::Frame, wizard: &Wizard, area: ratatui::layout::Rect) {
    match &wizard.step {
        Step::StorageBackend => {
            let items = vec![
                ListItem::new(" file  — local filesystem (default)"),
                ListItem::new(" s3    — S3-compatible object storage"),
            ];
            let mut state = wizard.list_state.clone();
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} ", wizard.step.title())),
                )
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");
            f.render_stateful_widget(list, area, &mut state);
        }
        Step::SmtpChoice => {
            let items = vec![
                ListItem::new(" Configure SMTP (enables invite email delivery)"),
                ListItem::new(" Skip SMTP (invite tokens printed to stdout instead)"),
            ];
            let mut state = wizard.list_state.clone();
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} ", wizard.step.title())),
                )
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");
            f.render_stateful_widget(list, area, &mut state);
        }
        Step::S3Config => render_s3_config(f, wizard, area),
        Step::SmtpConfig => render_smtp_config(f, wizard, area),
        Step::Confirm => render_confirm(f, wizard, area),
        Step::Done => render_done(f, area),
        _ => render_text_input(f, wizard, area),
    }
}

fn render_text_input(f: &mut ratatui::Frame, wizard: &Wizard, area: ratatui::layout::Rect) {
    let (prompt, placeholder, is_password) = match &wizard.step {
        Step::StoragePath => ("Local path for blob files:", "./shimmer-storage", false),
        Step::DbPath => ("SQLite database path:", "./shimmer-metadata.db", false),
        Step::OrgName => ("Organisation name:", "Acme Corp", false),
        Step::AdminEmail => ("Admin email address:", "admin@example.com", false),
        Step::AdminPassword => ("Admin password (min 8 chars):", "••••••••", true),
        _ => ("Input:", "", false),
    };

    let display_value = if is_password {
        "•".repeat(wizard.input.len())
    } else {
        wizard.input.clone()
    };

    let display_text = if display_value.is_empty() {
        Span::styled(placeholder, Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(display_value, Style::default().fg(Color::White))
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(area);

    let prompt_p = Paragraph::new(prompt).style(Style::default().fg(Color::Yellow));
    f.render_widget(prompt_p, chunks[0]);

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let input_p = Paragraph::new(Line::from(display_text)).block(input_block);
    f.render_widget(input_p, chunks[1]);

    if let Some(err) = &wizard.error {
        let err_p = Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red));
        f.render_widget(err_p, chunks[2]);
    }
}

fn render_s3_config(f: &mut ratatui::Frame, wizard: &Wizard, area: ratatui::layout::Rect) {
    // Parse current input
    let parts: Vec<&str> = wizard.input.splitn(4, '|').collect();
    let endpoint = parts.first().copied().unwrap_or("");
    let bucket = parts.get(1).copied().unwrap_or("shimmer");
    let access_key = parts.get(2).copied().unwrap_or("");
    let secret = parts.get(3).copied().unwrap_or("");
    let secret_display = "•".repeat(secret.len());

    let lines = vec![
        Line::from(vec![
            Span::styled("Endpoint:   ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if endpoint.is_empty() {
                    "http://localhost:9000"
                } else {
                    endpoint
                },
                if endpoint.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Bucket:     ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if bucket.is_empty() { "shimmer" } else { bucket },
                if bucket.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Access Key: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if access_key.is_empty() {
                    "(optional)"
                } else {
                    access_key
                },
                if access_key.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Secret Key: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if secret.is_empty() {
                    "(optional)"
                } else {
                    &secret_display
                },
                if secret.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
    ];

    let help_lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Enter values as:  endpoint|bucket|access_key|secret_key",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "Example: http://localhost:9000|shimmer|AKIAIOSFODNN7|wJalrXUtnFEMI",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let all_lines: Vec<Line> = lines.into_iter().chain(help_lines).collect();

    let p = Paragraph::new(all_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" S3 Configuration ")
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_smtp_config(f: &mut ratatui::Frame, wizard: &Wizard, area: ratatui::layout::Rect) {
    let parts: Vec<&str> = wizard.input.splitn(5, '|').collect();
    let host = parts.first().copied().unwrap_or("");
    let port = parts.get(1).copied().unwrap_or("587");
    let username = parts.get(2).copied().unwrap_or("");
    let password = parts.get(3).copied().unwrap_or("");
    let from = parts.get(4).copied().unwrap_or("");
    let pass_display = "•".repeat(password.len());

    let lines = vec![
        Line::from(vec![
            Span::styled("Host:     ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if host.is_empty() {
                    "smtp.example.com"
                } else {
                    host
                },
                if host.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Port:     ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if port.is_empty() { "587" } else { port },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("Username: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if username.is_empty() {
                    "(optional)"
                } else {
                    username
                },
                if username.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("Password: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if password.is_empty() {
                    "(optional)"
                } else {
                    &pass_display
                },
                if password.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("From:     ", Style::default().fg(Color::Yellow)),
            Span::styled(
                if from.is_empty() {
                    "noreply@example.com"
                } else {
                    from
                },
                if from.is_empty() {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                },
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Enter values as:  host|port|username|password|from",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SMTP Configuration ")
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_confirm(f: &mut ratatui::Frame, wizard: &Wizard, area: ratatui::layout::Rect) {
    let cfg = &wizard.config;
    let storage_detail = if cfg.storage_backend == "s3" {
        format!("  bucket: {}", cfg.s3_bucket)
    } else {
        format!("  path: {}", cfg.storage_path)
    };
    let smtp_line = if cfg.skip_smtp {
        "  (skipped — invite tokens will be printed to stdout)".into()
    } else {
        format!("  {}:{}", cfg.smtp_host, cfg.smtp_port)
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("Bind:        ", Style::default().fg(Color::Yellow)),
            Span::styled(cfg.bind.as_str(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Storage:     ", Style::default().fg(Color::Yellow)),
            Span::styled(
                cfg.storage_backend.as_str(),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(Span::styled(
            storage_detail,
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(vec![
            Span::styled("Database:    ", Style::default().fg(Color::Yellow)),
            Span::styled(cfg.db_path.as_str(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Org name:    ", Style::default().fg(Color::Yellow)),
            Span::styled(cfg.org_name.as_str(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Admin email: ", Style::default().fg(Color::Yellow)),
            Span::styled(cfg.admin_email.as_str(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("SMTP:        ", Style::default().fg(Color::Yellow)),
            Span::styled(smtp_line, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Config will be written to shimmer.toml (permissions: 0600)",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "A random JWT secret will be generated.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Review Configuration ")
                .border_style(Style::default().fg(Color::Green)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(p, area);
}

fn render_done(f: &mut ratatui::Frame, area: ratatui::layout::Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Setup complete!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("shimmer.toml has been written."),
        Line::from(""),
        Line::from("Next steps:"),
        Line::from("  1. Review shimmer.toml and adjust as needed."),
        Line::from("  2. Run:  shimmer-server serve"),
        Line::from("  3. The admin account is ready to log in."),
    ];
    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Done ")
                .border_style(Style::default().fg(Color::Green)),
        )
        .alignment(Alignment::Left);
    f.render_widget(p, area);
}

// ---------------------------------------------------------------------------
// Event handling
// ---------------------------------------------------------------------------

fn handle_key(wizard: &mut Wizard, key: crossterm::event::KeyEvent) {
    match &wizard.step {
        Step::StorageBackend | Step::SmtpChoice => {
            let item_count = 2;
            match key.code {
                KeyCode::Up => {
                    let i = wizard.list_state.selected().unwrap_or(0);
                    wizard.list_state.select(Some(i.saturating_sub(1)));
                }
                KeyCode::Down => {
                    let i = wizard.list_state.selected().unwrap_or(0);
                    wizard.list_state.select(Some((i + 1).min(item_count - 1)));
                }
                KeyCode::Enter => wizard.advance(),
                KeyCode::Esc => wizard.cancelled = true,
                _ => {}
            }
        }
        Step::Done => {
            // Any key exits
            wizard.cancelled = true; // signal loop to exit
        }
        _ => match key.code {
            KeyCode::Enter => wizard.advance(),
            KeyCode::Backspace => {
                wizard.input.pop();
            }
            KeyCode::Left => wizard.go_back(),
            KeyCode::Esc => wizard.cancelled = true,
            KeyCode::Char(c) => wizard.input.push(c),
            _ => {}
        },
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run the interactive TUI setup wizard.
///
/// Returns `Some(SetupConfig)` if the user completed the wizard, or `None` if
/// they cancelled (Esc / Ctrl-C).
///
/// # Errors
///
/// Returns an IO error if the terminal cannot be initialised.
pub fn run_setup_wizard() -> io::Result<Option<SetupConfig>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut wizard = Wizard::new();

    let result = run_loop(&mut terminal, &mut wizard);

    // Restore terminal regardless of error
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    match result {
        Ok(()) => {
            if wizard.cancelled || wizard.step != Step::Done {
                Ok(None)
            } else {
                Ok(Some(wizard.config))
            }
        }
        Err(e) => Err(e),
    }
}

fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    wizard: &mut Wizard,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| render(f, wizard))?;

        if wizard.cancelled {
            break;
        }
        if wizard.step == Step::Done {
            // Render one more frame to show the done screen, then wait for a key.
            terminal.draw(|f| render(f, wizard))?;
            loop {
                if event::poll(std::time::Duration::from_millis(200))? {
                    if let Event::Key(key) = event::read()? {
                        if key.kind == KeyEventKind::Press {
                            handle_key(wizard, key);
                            break;
                        }
                    }
                }
            }
            break;
        }

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(wizard, key);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Config file writing
// ---------------------------------------------------------------------------

/// Generate a cryptographically random 32-byte JWT secret, hex-encoded.
pub fn generate_jwt_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Write `shimmer.toml` from the collected `SetupConfig`.
///
/// The file is created with 0600 permissions on Unix systems so that only
/// the owning process user can read the JWT secret.
///
/// # Errors
///
/// Returns an IO error if the file cannot be written.
pub fn write_config_file(cfg: &SetupConfig, jwt_secret: &str) -> io::Result<()> {
    let mut content = format!(
        r#"# shimmer-server configuration
# Generated by `shimmer-server setup`

[server]
bind = "{bind}"
jwt_secret = "{jwt_secret}"

[storage]
backend = "{backend}"
"#,
        bind = cfg.bind,
        jwt_secret = jwt_secret,
        backend = cfg.storage_backend,
    );

    if cfg.storage_backend == "s3" {
        content.push_str(&format!(
            r#"
[storage.s3]
bucket = "{bucket}"
"#,
            bucket = cfg.s3_bucket,
        ));
        if !cfg.s3_endpoint.is_empty() {
            content.push_str(&format!("endpoint = \"{}\"\n", cfg.s3_endpoint));
        }
        if !cfg.s3_access_key.is_empty() {
            content.push_str(&format!("access_key_id = \"{}\"\n", cfg.s3_access_key));
        }
        if !cfg.s3_secret_key.is_empty() {
            content.push_str(&format!("secret_access_key = \"{}\"\n", cfg.s3_secret_key));
        }
    } else {
        content.push_str(&format!("path = \"{}\"\n", cfg.storage_path));
    }

    content.push_str(&format!(
        r#"
[database]
path = "{db_path}"

[org]
name = "{org_name}"
"#,
        db_path = cfg.db_path,
        org_name = cfg.org_name,
    ));

    if !cfg.skip_smtp {
        let port: u16 = cfg.smtp_port.parse().unwrap_or(587);
        content.push_str(&format!(
            r#"
[smtp]
host = "{host}"
port = {port}
username = "{username}"
password = "{password}"
from = "{from}"
"#,
            host = cfg.smtp_host,
            port = port,
            username = cfg.smtp_username,
            password = cfg.smtp_password,
            from = cfg.smtp_from,
        ));
    }

    // Write the file
    std::fs::write("shimmer.toml", &content)?;

    // Set 0600 permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions("shimmer.toml", perms)?;
    }

    Ok(())
}
