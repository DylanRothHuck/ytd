use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, BorderType, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

#[derive(PartialEq)]
enum AppState {
    InputPlaylistName,
    InputUrl,
    Downloading,
    Done,
    Error,
}

struct App {
    state: AppState,
    playlist_name: String,
    url: String,
    error_message: String,
    files_downloaded: Vec<String>,
    download_output: Arc<Mutex<String>>,
    download_output_final: String,
    download_done: Arc<AtomicBool>,
    download_success: Arc<AtomicBool>,
}

impl App {
    fn new() -> Self {
        Self {
            state: AppState::InputPlaylistName,
            playlist_name: String::new(),
            url: String::new(),
            error_message: String::new(),
            files_downloaded: Vec::new(),
            download_output: Arc::new(Mutex::new(String::new())),
            download_output_final: String::new(),
            download_done: Arc::new(AtomicBool::new(false)),
            download_success: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start_download(&mut self) {
        let music_dir = dirs::home_dir()
            .unwrap_or_default()
            .join("Music")
            .join(&self.playlist_name);

        let _ = std::fs::create_dir_all(&music_dir);

        let url = self.url.clone();
        let output_path = music_dir.display().to_string();
        let output_ref = self.download_output.clone();
        let done_ref = self.download_done.clone();
        let success_ref = self.download_success.clone();

        let output_clone = output_ref.clone();

        thread::spawn(move || {
            let mut child = Command::new("yt-dlp")
                .args([
                    "-f",
                    "ba[ext=m4a]",
                    "--extract-audio",
                    "--embed-thumbnail",
                    "--add-metadata",
                    "--convert-thumbnails",
                    "jpg",
                    "--output",
                    &format!("{}/%(title)s.%(ext)s", output_path),
                    &url,
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            match child {
                Ok(ref mut c) => {
                    let stdout = c.stdout.take();
                    let stderr = c.stderr.take();

                    let out1 = output_clone.clone();
                    let t1 = stdout.map(|s| {
                        thread::spawn(move || {
                            let reader = BufReader::new(s);
                            for line in reader.lines() {
                                if let Ok(l) = line {
                                    let mut out = out1.lock().unwrap();
                                    out.push_str(&l);
                                    out.push('\n');
                                }
                            }
                        })
                    });

                    let out2 = output_clone.clone();
                    let t2 = stderr.map(|s| {
                        thread::spawn(move || {
                            let reader = BufReader::new(s);
                            for line in reader.lines() {
                                if let Ok(l) = line {
                                    let mut out = out2.lock().unwrap();
                                    out.push_str(&l);
                                    out.push('\n');
                                }
                            }
                        })
                    });

                    if let Some(t) = t1 {
                        let _ = t.join();
                    }
                    if let Some(t) = t2 {
                        let _ = t.join();
                    }

                    let status = c.wait().unwrap_or_default();
                    success_ref.store(status.success(), Ordering::SeqCst);
                }
                Err(e) => {
                    let mut out = output_clone.lock().unwrap();
                    out.push_str(&format!("Failed to spawn: {}", e));
                    success_ref.store(false, Ordering::SeqCst);
                }
            }

            done_ref.store(true, Ordering::SeqCst);
        });
    }

    fn check_download(&mut self) -> bool {
        if self.download_done.load(Ordering::SeqCst) {
            self.download_output_final = {
                let out = self.download_output.lock().unwrap();
                out.clone()
            };

            if self.download_success.load(Ordering::SeqCst) {
                let music_dir = dirs::home_dir()
                    .unwrap_or_default()
                    .join("Music")
                    .join(&self.playlist_name);

                self.files_downloaded = std::fs::read_dir(&music_dir)
                    .ok()
                    .map(|d| {
                        d.filter_map(|e| e.ok())
                            .filter(|e| e.path().extension().map_or(false, |ext| ext == "m4a"))
                            .filter_map(|e| e.file_name().into_string().ok())
                            .collect()
                    })
                    .unwrap_or_default();

                self.state = AppState::Done;
            } else {
                self.error_message = "Download failed. Check your connection and URL.".to_string();
                self.state = AppState::Error;
            }
            return true;
        }
        false
    }
}

fn main() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let mut spinner_frame = 0u32;

    loop {
        terminal.draw(|f| ui(f, &mut app, spinner_frame))?;

        if app.state == AppState::Downloading {
            spinner_frame = spinner_frame.wrapping_add(1);

            if event::poll(std::time::Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    if key.code == KeyCode::Esc {
                        break;
                    }
                }
            }

            if app.check_download() {
                // Download finished
            }
            continue;
        }

        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                match app.state {
                    AppState::InputPlaylistName => {
                        if key.code == KeyCode::Enter {
                            if !app.playlist_name.is_empty() {
                                app.state = AppState::InputUrl;
                            }
                        } else if let KeyCode::Char(c) = key.code {
                            app.playlist_name.push(c);
                        } else if key.code == KeyCode::Backspace {
                            app.playlist_name.pop();
                        } else if key.code == KeyCode::Esc {
                            break;
                        }
                    }
                    AppState::InputUrl => {
                        if key.code == KeyCode::Enter {
                            if !app.url.is_empty() {
                                app.state = AppState::Downloading;
                                app.start_download();
                            }
                        } else if let KeyCode::Char(c) = key.code {
                            app.url.push(c);
                        } else if key.code == KeyCode::Backspace {
                            app.url.pop();
                        } else if key.code == KeyCode::Esc {
                            break;
                        }
                    }
                    AppState::Downloading => {
                        if app.check_download() {
                            // Download finished, state updated in check_download
                        }
                        if key.code == KeyCode::Esc {
                            break;
                        }
                    }
                    AppState::Done | AppState::Error => {
                        if key.code == KeyCode::Enter {
                            break;
                        }
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn ui(f: &mut Frame, app: &mut App, spinner_frame: u32) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(f.area());

    let title = Paragraph::new("YouTube Downloader TUI")
        .style(Style::default().fg(Color::Cyan))
        .block(Block::bordered().border_type(BorderType::Rounded))
        .alignment(Alignment::Center);
    f.render_widget(title, chunks[0]);

    match app.state {
        AppState::InputPlaylistName => {
            let name_input = Paragraph::new(app.playlist_name.as_str())
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title("Playlist Name"),
                )
                .style(Style::default().fg(Color::White));
            f.render_widget(name_input, chunks[1]);

            let hint = Paragraph::new("Enter playlist name, then press Enter")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(hint, chunks[2]);

            f.render_widget(
                Paragraph::new("").block(Block::bordered().border_type(BorderType::Rounded)),
                chunks[3],
            );
        }
        AppState::InputUrl => {
            let name_display = Paragraph::new(app.playlist_name.clone())
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title("Playlist Name"),
                )
                .style(Style::default().fg(Color::Green));
            f.render_widget(name_display, chunks[1]);

            let url_input = Paragraph::new(app.url.as_str())
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title("YouTube URL"),
                )
                .style(Style::default().fg(Color::White));
            f.render_widget(url_input, chunks[2]);

            let hint = Paragraph::new("Enter YouTube URL, then press Enter to download")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(hint, chunks[3]);
        }
        AppState::Downloading => {
            let name_display = Paragraph::new(app.playlist_name.clone())
                .block(
                    Block::bordered()
                        .border_type(BorderType::Rounded)
                        .title("Playlist Name"),
                )
                .style(Style::default().fg(Color::Green));
            f.render_widget(name_display, chunks[1]);

            let output = {
                let out = app.download_output.lock().unwrap();
                out.clone()
            };
            let output_display: String = output
                .lines()
                .rev()
                .take(5)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");

            let spinners = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
            let spinner = spinners[(spinner_frame as usize) % spinners.len()];

            let downloading =
                Paragraph::new(format!("{} Downloading...\n{}", spinner, output_display))
                    .style(Style::default().fg(Color::Yellow))
                    .block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .title("Progress"),
                    )
                    .alignment(Alignment::Center);
            f.render_widget(downloading, chunks[2]);

            f.render_widget(
                Paragraph::new("Press Esc to cancel")
                    .style(Style::default().fg(Color::DarkGray))
                    .alignment(Alignment::Center),
                chunks[3],
            );
        }
        AppState::Done => {
            let count = app.files_downloaded.len();
            let done = Paragraph::new(format!(
                "Download Complete! ({} file{})",
                count,
                if count == 1 { "" } else { "s" }
            ))
            .style(Style::default().fg(Color::Green))
            .block(Block::bordered().border_type(BorderType::Rounded))
            .alignment(Alignment::Center);
            f.render_widget(done, chunks[1]);

            let path = format!("~/Music/{}", app.playlist_name);
            let path_msg = Paragraph::new(format!("Saved to {}", path))
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Center);
            f.render_widget(path_msg, chunks[2]);

            if !app.files_downloaded.is_empty() {
                let files = app.files_downloaded.join("\n");
                let file_list = Paragraph::new(files)
                    .style(Style::default().fg(Color::DarkGray))
                    .block(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .title("Downloaded"),
                    )
                    .alignment(Alignment::Center);
                f.render_widget(file_list, chunks[3]);
            } else {
                let exit_hint = Paragraph::new("Press Enter to exit")
                    .style(Style::default().fg(Color::DarkGray))
                    .alignment(Alignment::Center);
                f.render_widget(exit_hint, chunks[3]);
            }
        }
        AppState::Error => {
            let error = Paragraph::new("Download Failed!")
                .style(Style::default().fg(Color::Red))
                .block(Block::bordered().border_type(BorderType::Rounded))
                .alignment(Alignment::Center);
            f.render_widget(error, chunks[1]);

            let error_msg = Paragraph::new(app.error_message.clone())
                .style(Style::default().fg(Color::Red))
                .block(Block::bordered().border_type(BorderType::Rounded))
                .alignment(Alignment::Center);
            f.render_widget(error_msg, chunks[2]);

            let exit_hint = Paragraph::new("Press Enter to exit")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(exit_hint, chunks[3]);
        }
    }
}
