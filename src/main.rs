use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, BorderType, Paragraph},
};
use std::env;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use std::collections::BTreeMap;

mod rss_funcs;
mod telegram_funcs;
use telegram_funcs::TelegramMonitor;

// --- UI Constants ---
const DARK_BG: Color = Color::Rgb(15, 15, 20);
const BORDER_MUTED: Color = Color::Rgb(50, 50, 60); 
const MATRIX_GREEN: Color = Color::Rgb(0, 235, 65);
const NEWS_GOLD: Color = Color::Rgb(255, 170, 50);
const SPORTS_CYAN: Color = Color::Rgb(0, 255, 255);
const WORLD_MAGENTA: Color = Color::Rgb(255, 0, 255);
const TELEGRAM_BLUE: Color = Color::Rgb(0, 136, 204);
const DESC_GREY: Color = Color::Rgb(120, 120, 130);
const UI_GREY: Color = Color::Rgb(160, 160, 170);

struct App {
    rss_feeds: Vec<Vec<(String, String, String)>>, 
    telegram_messages: BTreeMap<String, String>, 
    tx: mpsc::UnboundedSender<Vec<Vec<(String, String, String)>>>,
    rx: mpsc::UnboundedReceiver<Vec<Vec<(String, String, String)>>>,
    tg_rx: mpsc::UnboundedReceiver<(String, String)>, 
    offset: usize,
}

impl App {
    fn new(
        tx: mpsc::UnboundedSender<Vec<Vec<(String, String, String)>>>, 
        rx: mpsc::UnboundedReceiver<Vec<Vec<(String, String, String)>>>,
        tg_rx: mpsc::UnboundedReceiver<(String, String)>,
    ) -> Self {
        Self {
            // 0-2: Left Column | 3-5: Middle Column
            rss_feeds: vec![vec![]; 6],
            telegram_messages: BTreeMap::new(),
            tx,
            rx,
            tg_rx,
            offset: 0,
        }
    }

    fn on_tick(&mut self) {
        self.offset = self.offset.wrapping_add(1);
    }

    fn fetch_rss(&self) {
        let tx = self.tx.clone();
        let urls = vec![
            // Left Column (Tech)
            "https://feeds.feedburner.com/TheHackersNews",
            "https://www.computerweekly.com/rss/Latest-IT-news.xml",
            "https://sdtimes.com/feed/",
            // Middle Column (News)
            "https://www.investing.com/rss/news_25.rss",
            "https://www.channelnewsasia.com/api/v1/rss-outbound-feed?_format=xml",
            "https://www.channelnewsasia.com/api/v1/rss-outbound-feed?_format=xml&category=10416",
        ];

        tokio::spawn(async move {
            let mut categorized_feeds = Vec::new();
            for url in urls {
                if let Ok(feeds) = rss_funcs::get_feed(url).await {
                    categorized_feeds.push(feeds);
                } else {
                    categorized_feeds.push(vec![]); 
                }
            }
            let _ = tx.send(categorized_feeds);
        });
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    
    let api_id = env::var("TG_API_ID")?.parse::<i32>()?;
    let api_hash = env::var("TG_API_HASH")?;
    let target_ids: Vec<i64> = env::var("TG_CHAT_IDS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    let monitor = TelegramMonitor::new();
    let tg_client = monitor.create_client(api_id).await
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    monitor.ensure_authorized(&tg_client, &api_hash).await?;

    let (tx, rx) = mpsc::unbounded_channel();
    let (tg_tx, tg_rx) = mpsc::unbounded_channel();

    let mut app = App::new(tx, rx, tg_rx);
    app.fetch_rss();

    let ui_tg_tx = tg_tx.clone();
    tokio::spawn(async move {
        let _ = monitor.monitor(tg_client, target_ids, ui_tg_tx).await;
    });

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let tick_rate = Duration::from_secs(15);
    let mut last_tick = Instant::now();

    loop {
        while let Ok(new_feeds) = app.rx.try_recv() {
            app.rss_feeds = new_feeds;
            app.offset = 0;
        }
        while let Ok((sender, msg)) = app.tg_rx.try_recv() {
            app.telegram_messages.insert(sender, msg);
        }

        terminal.draw(|frame| {
            let area = frame.area();
            frame.render_widget(Block::default().bg(DARK_BG), area);

            let main_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(10), Constraint::Length(1)])
                .split(area);

            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(40), // Hacker/IT RSS
                    Constraint::Percentage(40), // General News RSS
                    Constraint::Percentage(20), // Telegram
                ])
                .split(main_layout[0]);

            // --- Column 1: Hacker/IT RSS (Stacked) ---
            let left_rss_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
                .split(columns[0]);

            let left_titles = [" THE HACKER NEWS ", " COMPUTER WEEKLY ", " SOFTWARE DEV TIMES "];
            for (idx, &sub_area) in left_rss_layout.iter().enumerate() {
                render_rss_block(frame, sub_area, &app, idx, left_titles[idx], MATRIX_GREEN, 2, None);
            }

            // --- Column 2: News RSS (Stacked) ---
            let mid_rss_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
                .split(columns[1]);

            let mid_configs = [
                (" STOCKS ", WORLD_MAGENTA, "Stocks"),
                (" WORLD NEWS", SPORTS_CYAN, "World"),
                (" LOCAL NEWS ", NEWS_GOLD, "Singapore"),
            ];

            for (i, &sub_area) in mid_rss_layout.iter().enumerate() {
                let (title, color, tag) = mid_configs[i];
                render_rss_block(frame, sub_area, &app, i + 3, title, color, 2, Some((tag, color)));
            }

            // --- Column 3: Telegram ---
            let tg_items: Vec<ListItem> = app.telegram_messages.iter().rev().take(20).map(|(s, m)| {
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(" ● ", Style::default().fg(TELEGRAM_BLUE)), 
                        Span::styled(s, Style::default().bold().fg(TELEGRAM_BLUE))
                    ]),
                    Line::from(vec![Span::raw("   "), Span::raw(m)]),
                    Line::from(""),
                ])
            }).collect();
            frame.render_widget(List::new(tg_items).block(create_block(" TELEGRAM ", TELEGRAM_BLUE)), columns[2]);

            // --- Footer ---
            let time_left = tick_rate.as_secs_f32() - last_tick.elapsed().as_secs_f32();
            let footer = Paragraph::new(Line::from(vec![
                Span::styled(" SYSTEM ", Style::default().bg(UI_GREY).fg(DARK_BG).bold()),
                Span::styled("", Style::default().fg(UI_GREY).bg(BORDER_MUTED)),
                Span::styled(" [Q] QUIT   [R] REFRESH ", Style::default().bg(BORDER_MUTED).fg(Color::White)),
                Span::styled("", Style::default().fg(BORDER_MUTED)),
                Span::raw(format!("   Syncing in: {:.0}s", time_left.max(0.0))),
            ]));
            frame.render_widget(footer, main_layout[1]);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let event::Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('r') => app.fetch_rss(),
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = Instant::now();
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn create_block<'a>(title: impl Into<Span<'a>>, color: Color) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER_MUTED))
        .title(title.into().patch_style(Style::default().fg(color).bold()))
}

fn render_rss_block(
    frame: &mut Frame, 
    area: Rect, 
    app: &App, 
    feed_idx: usize, 
    title: &str, 
    color: Color, 
    count: usize,
    tag_info: Option<(&str, Color)>
) {
    let mut items = Vec::new();
    let inner_width = (area.width as usize).saturating_sub(2);

    if let Some(feed) = app.rss_feeds.get(feed_idx) {
        if !feed.is_empty() {
            for i in 0..count {
                let item_idx = (app.offset + i) % feed.len();
                let (title_text, date, desc) = &feed[item_idx];
                
                let date_str = date.chars().take(10).collect::<String>();
                let label_prefix = "◆ ";
                
                // Calculate tag width if it exists
                let (tag_str, tag_color) = match tag_info {
                    Some((t, c)) => (format!(" [{}]", t), c),
                    None => ("".to_string(), Color::Reset),
                };

                let prefix_len = label_prefix.chars().count();
                let tag_len = tag_str.chars().count();
                
                // Max width title can take: Total - date - tag - prefix - padding
                let max_title_len = inner_width.saturating_sub(date_str.len() + tag_len + prefix_len + 2);
                let truncated_title = if title_text.chars().count() > max_title_len {
                    format!("{}...", title_text.chars().take(max_title_len.saturating_sub(3)).collect::<String>())
                } else {
                    title_text.clone()
                };

                // Alignment padding
                let current_content_len = prefix_len + truncated_title.chars().count() + date_str.len() + tag_len + 1;
                let padding = " ".repeat(inner_width.saturating_sub(current_content_len));

                let header_line = Line::from(vec![
                    Span::styled(label_prefix, Style::default().fg(color)),
                    Span::styled(truncated_title, Style::default().bold().fg(Color::White)),
                    Span::raw(padding),
                    Span::styled(date_str, Style::default().fg(DESC_GREY).italic()),
                    Span::styled(tag_str, Style::default().fg(tag_color).bold()),
                ]);

                let mut item_lines = vec![header_line];
                let clean_desc = desc.replace('\n', " ");
                for chunk in clean_desc.chars().collect::<Vec<char>>().chunks(inner_width).take(2) {
                    item_lines.push(Line::from(vec![
                        Span::styled(chunk.iter().collect::<String>(), Style::default().fg(DESC_GREY)),
                    ]));
                }

                items.push(ListItem::new(item_lines));
                
                if i < count - 1 {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled("─".repeat(inner_width), Style::default().fg(BORDER_MUTED))
                    ])));
                }
            }
        } else {
            items.push(ListItem::new("   Fetching data..."));
        }
    }
    frame.render_widget(List::new(items).block(create_block(title, color)), area);
}