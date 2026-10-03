use std::str::FromStr;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Gauge, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use wallet_lib::bitcoin::Address;

use crate::app::{App, Screen};
use crate::send_flow::{SendTracker, FINISHED, LAST_STAGE, STAGE_PAUSE, STAGES};

const ORANGE: Color = Color::Rgb(247, 147, 26);
const GREEN: Color = Color::Rgb(80, 220, 120);
const RED: Color = Color::Rgb(230, 80, 80);
const YELLOW: Color = Color::Rgb(230, 200, 80);
const CYAN: Color = Color::Rgb(110, 200, 255);
const DIM: Color = Color::DarkGray;
/// Secondary text: brighter than `DIM`, quieter than white.
const MUTED: Color = Color::Rgb(150, 150, 175);
/// Panel outlines: a muted orange so the bright accent stays for emphasis.
const BORDER: Color = Color::Rgb(125, 88, 42);

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

/// A card `percent_x` wide (but at least `min_width`) and exactly `height`
/// tall, centred in `area` and shrunk to fit if the area is smaller.
fn centered_fixed(percent_x: u16, min_width: u16, height: u16, area: Rect) -> Rect {
    let height = height.min(area.height);
    let width = (area.width * percent_x / 100).max(min_width).min(area.width);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// A rounded panel with a quiet outline and an accented title.
fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            format!(" ◆ {title} "),
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Left)
}

/// Format a satoshi amount as BTC with all 8 decimals, using integer math
/// so there's no float rounding.
fn btc(sats: u64) -> String {
    format!("{}.{:08} BTC", sats / 100_000_000, sats % 100_000_000)
}

/// `1234567` → `"1,234,567"`.
fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A BTC amount for display, with the leading zeros dimmed so the digits
/// that matter stand out: `0.00` dim, `150000` bright.
fn btc_spans(sats: u64, color: Color) -> Vec<Span<'static>> {
    let number = format!("{}.{:08}", sats / 100_000_000, sats % 100_000_000);
    let first_significant = number
        .char_indices()
        .find(|&(_, c)| c.is_ascii_digit() && c != '0')
        .map_or(number.len(), |(i, _)| i);
    let (zeros, rest) = number.split_at(first_significant);

    vec![
        Span::styled(zeros.to_string(), Style::default().fg(DIM)),
        Span::styled(rest.to_string(), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(" BTC", Style::default().fg(MUTED)),
    ]
}

/// Shorten an ASCII address to at most `max` characters, keeping both ends.
fn shorten(address: &str, max: usize) -> String {
    let max = max.max(12);
    if address.len() <= max {
        return address.to_string();
    }
    let keep = max - 1;
    let head = keep * 2 / 3;
    let tail = keep - head;
    format!("{}…{}", &address[..head], &address[address.len() - tail..])
}

/// A key shown as a badge, followed by what it does.
fn hint(key: char, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {key} "),
            Style::default().fg(Color::Black).bg(ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label}    "), Style::default().fg(MUTED)),
    ]
}

/// A one-line status message with an icon, coloured by outcome.
fn status_line(message: &str, is_error: bool) -> Line<'static> {
    let (icon, color) = if is_error { ("✘", RED) } else { ("✔", GREEN) };
    Line::from(vec![
        Span::styled(format!("{icon} "), Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::styled(message.to_string(), Style::default().fg(color)),
    ])
}

pub fn draw(frame: &mut Frame, app: &App) {
    match &app.screen {
        Screen::Welcome => draw_welcome(frame),
        Screen::ShowMnemonic(words) => draw_mnemonic(frame, words, &app.status, app.status_is_error),
        Screen::RestoreInput(words) => draw_restore(frame, words),
        Screen::Main => draw_main(frame, app),
        Screen::Sending => draw_sending(frame, app),
        Screen::Send { address, amount, editing_address } => draw_send(
            frame,
            address,
            amount,
            *editing_address,
            &app.status,
            app.status_is_error,
            app.balance.0 + app.balance.1,
        ),
    }
}

/// Five-row block letters for the splash logo.
fn block_letter(c: char) -> [&'static str; 5] {
    match c {
        'W' => ["█   █", "█   █", "█ █ █", "██ ██", "█   █"],
        'A' => [" ███ ", "█   █", "█████", "█   █", "█   █"],
        'L' => ["█    ", "█    ", "█    ", "█    ", "█████"],
        'E' => ["█████", "█    ", "████ ", "█    ", "█████"],
        'T' => ["█████", "  █  ", "  █  ", "  █  ", "  █  "],
        _ => ["     "; 5],
    }
}

fn draw_welcome(frame: &mut Frame) {
    // The logo fades from bright to deep orange, top to bottom.
    const FADE: [Color; 5] = [
        Color::Rgb(255, 184, 77),
        Color::Rgb(252, 166, 48),
        Color::Rgb(247, 147, 26),
        Color::Rgb(220, 124, 18),
        Color::Rgb(184, 100, 14),
    ];

    let area = centered_fixed(60, 46, 16, frame.area());

    let mut lines = vec![Line::from("")];
    for (row, color) in FADE.iter().enumerate() {
        let text = "WALLET"
            .chars()
            .map(|c| block_letter(c)[row])
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(Line::from(Span::styled(text, Style::default().fg(*color))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("₿ ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
        Span::styled("Bitcoin wallet", Style::default().fg(MUTED)),
    ]));
    lines.push(Line::from(Span::styled(
        "━━━━━━━  Regtest  ━━━━━━━",
        Style::default().fg(BORDER),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(""));

    let mut keys = hint('n', "New wallet");
    keys.extend(hint('r', "Restore"));
    keys.extend(hint('q', "Quit"));
    lines.push(Line::from(keys));

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .block(panel("Welcome")),
        area,
    );
}

fn draw_mnemonic(frame: &mut Frame, words: &str, status: &str, status_is_error: bool) {
    let area = centered_rect(72, 72, frame.area());
    let block = panel("Your Recovery Phrase");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // warning
            Constraint::Min(6),    // word grid
            Constraint::Length(2), // status
            Constraint::Length(2), // hints
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "⚠  Write these down on paper. Never share them. They cannot be recovered.",
            Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true }),
        rows[0],
    );

    let word_list: Vec<&str> = words.split_whitespace().collect();
    // Each word gets its own little card when there's room for them;
    // otherwise fall back to a plain grid.
    let cards = rows[1].height >= 12;
    let grid = if cards {
        let top = rows[1].y + (rows[1].height - 12) / 2;
        Rect::new(rows[1].x, top, rows[1].width, 12)
    } else {
        rows[1]
    };
    let word_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if cards {
            [Constraint::Length(3); 4]
        } else {
            [Constraint::Ratio(1, 4); 4]
        })
        .split(grid);

    for (row_i, row_area) in word_rows.iter().enumerate() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 3); 3])
            .split(*row_area);

        for (col_i, cell_area) in cols.iter().enumerate() {
            let index = row_i * 3 + col_i;
            let Some(word) = word_list.get(index) else {
                continue;
            };

            let line = Line::from(vec![
                Span::styled(format!("{:>2}  ", index + 1), Style::default().fg(ORANGE)),
                Span::styled(
                    word.to_string(),
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
                ),
            ]);

            if cards {
                let card = Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(BORDER));
                let card_inner = card.inner(*cell_area);
                frame.render_widget(card, *cell_area);
                frame.render_widget(
                    Paragraph::new(line).alignment(Alignment::Center),
                    card_inner,
                );
            } else {
                frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), *cell_area);
            }
        }
    }

    if !status.is_empty() {
        frame.render_widget(
            Paragraph::new(status_line(status, status_is_error)).alignment(Alignment::Center),
            rows[2],
        );
    }

    let mut keybinds = hint('c', "Copy to clipboard");
    keybinds.extend(hint('↵', "Continue"));
    frame.render_widget(
        Paragraph::new(Line::from(keybinds)).alignment(Alignment::Center),
        rows[3],
    );
}

fn draw_restore(frame: &mut Frame, words: &str) {
    let area = centered_rect(64, 36, frame.area());
    let block = panel("Restore Wallet");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(3), Constraint::Length(2)])
        .split(inner);

    let count = words.split_whitespace().count();
    let (count_color, count_note) = match count {
        12 => (GREEN, "ready to restore"),
        _ => (MUTED, "words"),
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("Enter your 12-word recovery phrase", Style::default().fg(MUTED))),
            Line::from(Span::styled(
                format!("{count} / 12 {count_note}"),
                Style::default().fg(count_color),
            )),
        ])
        .alignment(Alignment::Center),
        rows[0],
    );

    let input_box = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ORANGE));
    let input_inner = input_box.inner(rows[1]);
    frame.render_widget(input_box, rows[1]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(words.to_string(), Style::default().fg(Color::White)),
            Span::styled("▏", Style::default().fg(ORANGE)),
        ]))
        .wrap(Wrap { trim: false }),
        input_inner,
    );

    frame.render_widget(
        Paragraph::new(Line::from(hint('↵', "Restore"))).alignment(Alignment::Center),
        rows[2],
    );
}

fn draw_main(frame: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header
            Constraint::Min(9),    // body
            Constraint::Length(1), // status / sync progress
            Constraint::Length(1), // key hints
        ])
        .margin(1)
        .split(frame.area());

    draw_header(frame, app, rows[0]);

    // Balance and the receive address side by side, the outputs beneath.
    let top = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(5)])
        .split(rows[1]);
    let summary = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(top[0]);

    draw_balance(frame, app, summary[0]);
    draw_receive(frame, app, summary[1]);
    draw_spendable(frame, app, top[1]);

    if app.is_syncing() {
        draw_sync_progress(frame, app, rows[2]);
    } else if !app.status.is_empty() {
        frame.render_widget(
            Paragraph::new(status_line(&app.status, app.status_is_error)).alignment(Alignment::Center),
            rows[2],
        );
    }

    let mut keybinds = hint('c', "Copy address");
    keybinds.extend(hint('a', "New address"));
    keybinds.extend(hint('s', "Sync"));
    keybinds.extend(hint('p', "Send"));
    keybinds.extend(hint('q', "Quit"));
    frame.render_widget(Paragraph::new(Line::from(keybinds)).alignment(Alignment::Center), rows[3]);
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let rule = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(BORDER));
    let inner = rule.inner(area);
    frame.render_widget(rule, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(26)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ₿ ", Style::default().fg(Color::Black).bg(ORANGE).add_modifier(Modifier::BOLD)),
            Span::styled(" ", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
        ])),
        cols[0],
    );

    let mut right = Vec::new();
    if app.is_syncing() {
        right.push(Span::styled("⟳ syncing  ", Style::default().fg(ORANGE)));
    }
    right.push(Span::styled(
        " REGTEST ",
        Style::default().fg(Color::Black).bg(CYAN).add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        cols[1],
    );
}

fn draw_balance(frame: &mut Frame, app: &App, area: Rect) {
    let (confirmed, unconfirmed) = app.balance;

    let mut lines = vec![Line::from(""), Line::from(btc_spans(confirmed, GREEN))];
    lines.push(Line::from(Span::styled(
        format!("{} sats", group_digits(confirmed)),
        Style::default().fg(MUTED),
    )));
    lines.push(if unconfirmed > 0 {
        Line::from(Span::styled(
            format!("◌ +{} pending", btc(unconfirmed)),
            Style::default().fg(YELLOW),
        ))
    } else {
        Line::from(Span::styled("● all confirmed", Style::default().fg(DIM)))
    });

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .block(panel("Balance")),
        area,
    );
}

/// The current receive address, with the key that copies it.
fn draw_receive(frame: &mut Frame, app: &App, area: Rect) {
    let mut lines = vec![Line::from("")];

    match app.address.as_deref() {
        Some(address) => {
            lines.push(Line::from(Span::styled(
                address.to_string(),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(vec![
                Span::styled("press ", Style::default().fg(DIM)),
                Span::styled(" c ", Style::default().fg(Color::Black).bg(ORANGE).add_modifier(Modifier::BOLD)),
                Span::styled(" to copy · ", Style::default().fg(DIM)),
                Span::styled(" a ", Style::default().fg(Color::Black).bg(ORANGE).add_modifier(Modifier::BOLD)),
                Span::styled(" for a fresh one", Style::default().fg(DIM)),
            ]));
        }
        None => lines.push(Line::from(Span::styled("(no address yet)", Style::default().fg(DIM)))),
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(panel("Receive")),
        area,
    );
}

/// Draws the sync's indicatif progress bar as a one-line gauge.
fn draw_sync_progress(frame: &mut Frame, app: &App, area: Rect) {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let bar = &app.progress;
    let position = bar.position();
    let length = bar.length().unwrap_or(0);
    let elapsed = bar.elapsed();

    let ratio = if length == 0 {
        0.0
    } else {
        (position as f64 / length as f64).clamp(0.0, 1.0)
    };
    let spinner = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];
    let label = format!(
        "{spinner} {}  {position}/{length}  {:.1}s",
        bar.message(),
        elapsed.as_secs_f32()
    );

    frame.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(ORANGE).bg(Color::Rgb(45, 34, 24)))
            .use_unicode(true)
            .ratio(ratio)
            .label(label),
        area,
    );
}

/// Every address that holds spendable outputs as of the last sync, with how
/// many outputs it has and their total value.
fn draw_spendable(frame: &mut Frame, app: &App, area: Rect) {
    // (address, output count, confirmed sats, unconfirmed sats), in the
    // order each address first appears among the wallet's UTXOs.
    let mut by_address: Vec<(String, usize, u64, u64)> = Vec::new();

    for utxo in &app.utxos {
        let address = utxo.address.to_string();
        let sats = utxo.amount.to_sat();

        let entry = match by_address.iter().position(|(a, ..)| *a == address) {
            Some(i) => &mut by_address[i],
            None => {
                by_address.push((address, 0, 0, 0));
                by_address.last_mut().unwrap()
            }
        };

        entry.1 += 1;
        if utxo.confirmed {
            entry.2 += sats;
        } else {
            entry.3 += sats;
        }
    }

    let title = format!("Spendable Outputs · {}", by_address.len());
    let block = panel(&title);

    if by_address.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("No spendable outputs yet", Style::default().fg(MUTED))),
                Line::from(vec![
                    Span::styled("press ", Style::default().fg(DIM)),
                    Span::styled(" s ", Style::default().fg(Color::Black).bg(ORANGE)),
                    Span::styled(" to sync", Style::default().fg(DIM)),
                ]),
            ])
            .alignment(Alignment::Center)
            .block(block),
            area,
        );
        return;
    }

    const COUNT_W: u16 = 5;
    const CONFIRMED_W: u16 = 16;
    const PENDING_W: u16 = 17;
    let spacing = 2u16;
    // Confirmed and pending coins are kept apart so they can't be mistaken
    // for each other. A narrow table folds pending into a marker instead.
    let show_pending = area.width >= 70;
    let columns = if show_pending { 4 } else { 3 };
    let fixed = COUNT_W + CONFIRMED_W + if show_pending { PENDING_W } else { 0 } + (columns - 1) * spacing;
    let address_w = area.width.saturating_sub(2 + 1 + fixed) as usize;

    let header_cell = |text: &'static str, alignment: Alignment| {
        Cell::from(Line::from(text).alignment(alignment))
            .style(Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))
    };
    let mut header_cells = vec![
        header_cell("ADDRESS", Alignment::Left),
        header_cell("UTXOS", Alignment::Right),
        header_cell("CONFIRMED", Alignment::Right),
    ];
    if show_pending {
        header_cells.push(header_cell("PENDING", Alignment::Right));
    }
    let header = Row::new(header_cells).bottom_margin(1);

    let rows: Vec<Row> = by_address
        .iter()
        .map(|(address, count, confirmed, unconfirmed)| {
            let mut confirmed_spans = if *confirmed > 0 {
                btc_spans(*confirmed, GREEN)
            } else {
                vec![Span::styled("—", Style::default().fg(DIM))]
            };
            if !show_pending && *unconfirmed > 0 {
                confirmed_spans.push(Span::styled(" ◌", Style::default().fg(YELLOW)));
            }

            let mut cells = vec![
                Cell::from(shorten(address, address_w)).style(Style::default().fg(Color::White)),
                Cell::from(Line::from(count.to_string()).alignment(Alignment::Right))
                    .style(Style::default().fg(MUTED)),
                Cell::from(Line::from(confirmed_spans).alignment(Alignment::Right)),
            ];
            if show_pending {
                let pending_spans = if *unconfirmed > 0 {
                    let mut spans = vec![Span::styled("+", Style::default().fg(YELLOW))];
                    spans.extend(btc_spans(*unconfirmed, YELLOW));
                    spans
                } else {
                    vec![Span::styled("—", Style::default().fg(DIM))]
                };
                cells.push(Cell::from(Line::from(pending_spans).alignment(Alignment::Right)));
            }
            Row::new(cells)
        })
        .collect();

    let mut widths = vec![
        Constraint::Fill(1),
        Constraint::Length(COUNT_W),
        Constraint::Length(CONFIRMED_W),
    ];
    if show_pending {
        widths.push(Constraint::Length(PENDING_W));
    }

    frame.render_widget(
        Table::new(rows, widths)
        .header(header)
        .column_spacing(spacing)
        .block(block),
        area,
    );
}

/// Follows a send from PSBT to confirmation: a progress bar over the five
/// stages, a checklist of them, and the transaction's details.
fn draw_sending(frame: &mut Frame, app: &App) {
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    let Some(send) = &app.send else {
        return;
    };

    let area = centered_rect(85, 88, frame.area());
    let block = panel("Sending");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(STAGES.len() as u16 + 1),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(inner);

    let position = send.bar.position();
    let elapsed = send.bar.elapsed();
    let spinner = SPINNER[(elapsed.as_millis() / 100) as usize % SPINNER.len()];

    let (bar_color, label) = if send.error.is_some() {
        (RED, "failed".to_string())
    } else if send.is_finished() {
        (GREEN, "confirmed".to_string())
    } else {
        (ORANGE, format!("{position}/{FINISHED}  {:.1}s", elapsed.as_secs_f32()))
    };
    frame.render_widget(
        Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(DIM)),
            )
            .gauge_style(Style::default().fg(bar_color).bg(Color::DarkGray))
            .ratio((position as f64 / FINISHED as f64).clamp(0.0, 1.0))
            .label(label),
        rows[0],
    );

    let stage_lines: Vec<Line> = STAGES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let i = i as u64;
            let (mark, color) = if send.error.is_some() && i == position {
                ("✘".to_string(), RED)
            } else if i < position {
                ("✔".to_string(), GREEN)
            } else if i == position {
                (spinner.to_string(), ORANGE)
            } else {
                ("○".to_string(), DIM)
            };

            let mut spans = vec![
                Span::styled(format!(" {mark} "), Style::default().fg(color)),
                Span::styled(*name, Style::default().fg(if i > position { DIM } else { Color::White })),
            ];
            if i == LAST_STAGE as u64 && position == i && send.error.is_none() {
                spans.push(Span::styled(
                    "  — waiting for confirmation",
                    Style::default().fg(YELLOW),
                ));
            } else if i == LAST_STAGE as u64 && send.is_finished() {
                spans.push(Span::styled(
                    format!("  ({} confirmation(s))", send.confirmations),
                    Style::default().fg(DIM),
                ));
            }
            Line::from(spans)
        })
        .collect();
    draw_tx_animation(frame, send, rows[1]);
    frame.render_widget(Paragraph::new(Text::from(stage_lines)), rows[2]);

    let mut details = Vec::new();
    if let Some(summary) = &send.summary {
        details.push(Line::from(vec![
            Span::styled(" PSBT  ", Style::default().fg(DIM)),
            Span::styled(summary.as_str(), Style::default().fg(Color::White)),
        ]));
    }
    if let Some(txid) = &send.txid {
        details.push(Line::from(vec![
            Span::styled(" txid  ", Style::default().fg(DIM)),
            Span::styled(txid.to_string(), Style::default().fg(Color::White)),
        ]));
    }
    if let Some(error) = &send.error {
        details.push(Line::from(Span::styled(
            format!(" {error}"),
            Style::default().fg(RED),
        )));
    }
    frame.render_widget(
        Paragraph::new(Text::from(details)).wrap(Wrap { trim: false }),
        rows[3],
    );

    // Until the wallet is handed back, leaving would strand it.
    let hints = if app.wallet.is_none() {
        Line::from(Span::styled("working…", Style::default().fg(DIM)))
    } else if send.error.is_some() {
        Line::from(hint('⎋', "Back to form"))
    } else {
        Line::from(hint('⎋', "Done"))
    };
    frame.render_widget(Paragraph::new(hints).alignment(Alignment::Center), rows[4]);
}

/// Where the transaction "token" is along the diagram, from 0.0 (at the
/// wallet) to 4.0 (in a block). It travels one link per active stage, then
/// waits in the mempool until a block takes it.
fn token_position(send: &SendTracker) -> f32 {
    let stage = send.bar.position();

    // Ease in and out of each hop so it rests briefly at both ends. A
    // failed send freezes the token where it was.
    let hop = if send.error.is_some() {
        0.0
    } else {
        let raw = send.stage_elapsed().as_secs_f32() / STAGE_PAUSE.as_secs_f32();
        ((raw - 0.1) / 0.8).clamp(0.0, 1.0)
    };

    match stage {
        0 => 0.0,
        1..=3 => (stage - 1) as f32 + hop,
        4 | 5 => 3.0,
        _ => {
            let since = send.finished_at.map_or(0.0, |t| t.elapsed().as_secs_f32());
            3.0 + (since / 0.9).clamp(0.0, 1.0)
        }
    }
}

/// Draws the transaction travelling Wallet → PSBT → Signed tx → Mempool →
/// Block, in step with the real stages of the send.
fn draw_tx_animation(frame: &mut Frame, send: &SendTracker, area: Rect) {
    const NODE_W: usize = 11;
    const LABELS: [&str; 5] = ["Wallet", "PSBT", "Signed tx", "Mempool", "Block"];
    const CAPTIONS: [&str; 5] = ["keys+coins", "unsigned", "signed", "0 conf", "1 conf"];
    type Cell = (char, Style);

    fn put(grid: &mut [Vec<Cell>], row: usize, col: usize, ch: char, style: Style) {
        if let Some(cell) = grid.get_mut(row).and_then(|r| r.get_mut(col)) {
            *cell = (ch, style);
        }
    }

    fn put_str(grid: &mut [Vec<Cell>], row: usize, col: usize, text: &str, style: Style) {
        for (i, ch) in text.chars().enumerate() {
            put(grid, row, col + i, ch, style);
        }
    }

    let width = area.width as usize;
    if width < 5 * NODE_W + 4 * 2 || area.height < 4 {
        return;
    }

    let link_w = ((width - 5 * NODE_W) / 4).min(10);
    let left = (width - (5 * NODE_W + 4 * link_w)) / 2;
    let tick = send.bar.elapsed().as_millis() as usize;

    let pos = token_position(send);
    let failed = send.error.is_some();
    let finished = send.is_finished();
    let waiting = send.bar.position() as usize == LAST_STAGE && !failed && !finished;
    let arrived_final = finished && pos >= 3.999;
    let last_reached = (0..5).rev().find(|&i| pos >= i as f32 - 0.001).unwrap_or(0);

    let mut grid: Vec<Vec<Cell>> = vec![vec![(' ', Style::default()); width]; 4];

    for i in 0..5 {
        let x = left + i * (NODE_W + link_w);
        let reached = i <= last_reached;
        let color = if failed && i == last_reached {
            RED
        } else if i < last_reached || (arrived_final && i == last_reached) {
            GREEN
        } else if i == last_reached {
            ORANGE
        } else {
            DIM
        };
        let border = Style::default().fg(color);
        let inner = NODE_W - 2;

        put_str(&mut grid, 0, x, &format!("╭{}╮", "─".repeat(inner)), border);
        put_str(&mut grid, 2, x, &format!("╰{}╯", "─".repeat(inner)), border);
        put(&mut grid, 1, x, '│', border);
        put(&mut grid, 1, x + NODE_W - 1, '│', border);

        let label = LABELS[i];
        let pad = (inner - label.len()) / 2;
        let label_style = if reached {
            Style::default().fg(color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(DIM)
        };
        put_str(&mut grid, 1, x + 1 + pad, label, label_style);

        let (caption, caption_style) = if i == 4 && waiting {
            ("waiting…", Style::default().fg(YELLOW))
        } else if reached {
            (CAPTIONS[i], Style::default().fg(Color::White))
        } else {
            (CAPTIONS[i], Style::default().fg(DIM))
        };
        let caption_x = x + NODE_W.saturating_sub(caption.chars().count()) / 2;
        put_str(&mut grid, 3, caption_x, caption, caption_style);
    }

    for i in 0..4 {
        let x = left + i * (NODE_W + link_w) + NODE_W;
        let progress = (pos - i as f32).clamp(0.0, 1.0);
        let travelled = (progress * link_w as f32).round() as usize;
        let flight_color = if progress >= 1.0 { GREEN } else { ORANGE };

        for k in 0..link_w {
            let ch = if k + 1 == link_w { '▶' } else { '─' };
            let style = if k < travelled {
                Style::default().fg(flight_color)
            } else {
                Style::default().fg(DIM)
            };
            put(&mut grid, 1, x + k, ch, style);
        }

        // Waiting for a block: dots drift along the mempool → block link.
        if waiting && i == 3 {
            for k in 0..link_w.saturating_sub(1) {
                if (k + tick / 120) % 4 == 0 {
                    put(&mut grid, 1, x + k, '•', Style::default().fg(YELLOW));
                }
            }
        }

        if progress > 0.0 && progress < 1.0 {
            let k = (progress * (link_w - 1) as f32).round() as usize;
            put(&mut grid, 1, x + k, '●', Style::default().fg(ORANGE).add_modifier(Modifier::BOLD));
        }
    }

    let lines: Vec<Line> = grid
        .into_iter()
        .map(|row| {
            Line::from(
                row.into_iter()
                    .map(|(ch, style)| Span::styled(ch.to_string(), style))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_send(
    frame: &mut Frame,
    address: &str,
    amount: &str,
    editing_address: bool,
    status: &str,
    status_is_error: bool,
    available: u64,
) {
    let area = centered_fixed(64, 50, 16, frame.area());
    let block = panel("Send");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // available balance
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(inner);

    let mut available_line = vec![Span::styled("Available  ", Style::default().fg(DIM))];
    available_line.extend(btc_spans(available, GREEN));
    frame.render_widget(
        Paragraph::new(Line::from(available_line)).alignment(Alignment::Center),
        rows[0],
    );

    draw_field(frame, rows[1], rows[2], "Address", address, editing_address);

    // The dust limit depends on the kind of address being paid; assume the
    // common native SegWit one until the typed address parses.
    let dust_limit = Address::from_str(address)
        .map(|a| a.assume_checked().script_pubkey().minimal_non_dust().to_sat())
        .unwrap_or(294);
    let amount_label = match amount.parse::<u64>() {
        Ok(sats) if sats < dust_limit => {
            format!("Amount (sats)  ≈ {}  ⚠ dust, minimum {dust_limit}", btc(sats))
        }
        Ok(sats) => format!("Amount (sats)  ≈ {}", btc(sats)),
        Err(_) => "Amount (sats)".to_string(),
    };
    draw_field(frame, rows[3], rows[4], &amount_label, amount, !editing_address);

    if !status.is_empty() {
        frame.render_widget(
            Paragraph::new(status_line(status, status_is_error))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            rows[5],
        );
    }

    let mut keybinds = hint('⇥', "Switch field");
    keybinds.extend(hint('↵', "Send"));
    keybinds.extend(hint('⎋', "Cancel"));
    frame.render_widget(Paragraph::new(Line::from(keybinds)).alignment(Alignment::Center), rows[6]);
}

fn draw_field(frame: &mut Frame, label_area: Rect, box_area: Rect, label: &str, value: &str, active: bool) {
    let (label_color, border_color) = if active { (ORANGE, ORANGE) } else { (MUTED, BORDER) };

    frame.render_widget(
        Paragraph::new(Span::styled(label, Style::default().fg(label_color))),
        label_area,
    );

    let field_box = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));
    let field_inner = field_box.inner(box_area);
    frame.render_widget(field_box, box_area);

    let mut spans = vec![Span::styled(value.to_string(), Style::default().fg(Color::White))];
    if active {
        spans.push(Span::styled("▏", Style::default().fg(ORANGE)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }), field_inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::RpcConfig;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::str::FromStr;
    use wallet_lib::backend::Utxo;
    use wallet_lib::bitcoin::{Amount, OutPoint, Txid};

    const ADDR_A: &str = "bcrt1qy0rdmetap73wehxu4pd9l8fr752nmsnsmduv2v";
    const ADDR_B: &str = "bcrt1q04fhrz8lsugs4r94u8rr4jyvh6qxplnkd55v38";

    fn app() -> App {
        App::new(RpcConfig {
            url: String::new(),
            user: String::new(),
            pass: String::new(),
        })
    }

    fn utxo(address: &str, sats: u64, confirmed: bool) -> Utxo {
        let txid = Txid::from_str(&"0".repeat(64)).unwrap();
        Utxo {
            outpoint: OutPoint::new(txid, 0),
            address: Address::from_str(address).unwrap().assume_checked(),
            amount: Amount::from_sat(sats),
            confirmed,
        }
    }

    /// Render `app` at the given size and return the screen as text.
    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();

        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn digits_are_grouped_and_amounts_formatted() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(139_999_164), "139,999,164");
        assert_eq!(btc(1_379_999_718), "13.79999718 BTC");
    }

    #[test]
    fn long_addresses_are_shortened_to_fit() {
        assert_eq!(shorten(ADDR_A, 60), ADDR_A);
        let short = shorten(ADDR_A, 20);
        assert_eq!(short.chars().count(), 20);
        assert!(short.starts_with("bcrt1qy0rdm"));
        assert!(short.ends_with("duv2v"));
    }

    #[test]
    fn welcome_shows_the_logo_and_keys() {
        let screen = render(&app(), 90, 30);
        println!("{screen}");
        assert!(screen.contains("█████"));
        assert!(screen.contains("New wallet"));
    }

    #[test]
    fn main_shows_balance_outputs_and_receive_address() {
        let mut app = app();
        app.screen = Screen::Main;
        app.address = Some(ADDR_A.to_string());
        app.balance = (139_999_164, 50_000_000);
        app.utxos = vec![
            utxo(ADDR_A, 50_000_000, true),
            utxo(ADDR_B, 29_999_582, true),
            utxo(ADDR_B, 9_999_582, false),
        ];
        app.set_ok("Synced.");

        let screen = render(&app, 120, 40);
        println!("{screen}");
        assert!(screen.contains("1.39999164"));
        assert!(screen.contains("139,999,164 sats"));
        assert!(screen.contains("Spendable Outputs · 2"));
        assert!(screen.contains(ADDR_A));
        assert!(screen.contains("to copy"));
        // The second address holds 0.29999582 confirmed and 0.09999582 pending:
        // each shows in its own column and they are never added together.
        assert!(screen.contains("0.29999582 BTC"));
        assert!(screen.contains("+0.09999582 BTC"));
        assert!(!screen.contains("0.39999164"));
        assert!(screen.contains("Synced."));
    }

    #[test]
    fn main_still_draws_on_a_small_terminal() {
        let mut app = app();
        app.screen = Screen::Main;
        app.address = Some(ADDR_A.to_string());
        app.utxos = vec![utxo(ADDR_A, 50_000_000, true)];

        let screen = render(&app, 80, 24);
        println!("{screen}");
        assert!(screen.contains("bcrt1q"));
        assert!(screen.contains("Copy address"));
    }

    #[test]
    fn mnemonic_screen_draws_all_twelve_words() {
        let mut app = app();
        let words = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
        app.screen = Screen::ShowMnemonic(words.to_string());
        app.set_ok("Copied to clipboard.");

        let screen = render(&app, 100, 36);
        println!("{screen}");
        for word in words.split_whitespace() {
            assert!(screen.contains(word), "missing {word}");
        }
        assert!(screen.contains("Copied to clipboard."));
    }

    #[test]
    fn restore_counts_words() {
        let mut app = app();
        app.screen = Screen::RestoreInput("alpha bravo charlie".to_string());
        let screen = render(&app, 90, 30);
        println!("{screen}");
        assert!(screen.contains("3 / 12"));
    }

    #[test]
    fn send_form_warns_about_dust_and_shows_available_balance() {
        let mut app = app();
        app.balance = (139_999_164, 0);
        app.screen = Screen::Send {
            address: ADDR_B.to_string(),
            amount: "100".to_string(),
            editing_address: false,
        };
        let screen = render(&app, 100, 36);
        println!("{screen}");
        assert!(screen.contains("dust, minimum 294"));
        assert!(screen.contains("1.39999164"));
    }
}
