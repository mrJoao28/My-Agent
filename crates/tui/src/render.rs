use crate::{
    app::{App, Screen},
    theme::{Palette, format_tokens, token_ratio},
    transcript::{Entry, Speaker},
};
use myagent_protocol::Mode;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, Paragraph, Wrap},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Plan => "PLAN",
        Mode::Build => "BUILD",
        _ => "?",
    }
}
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = vec![];
    for line in text.split('\n') {
        let mut part = String::new();
        let mut used = 0;
        for g in line.graphemes(true) {
            let w = g.width();
            if used + w > width && !part.is_empty() {
                out.push(std::mem::take(&mut part));
                used = 0;
            }
            part.push_str(g);
            used += w;
        }
        out.push(part);
    }
    out
}
fn clipped(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        if used + g.width() > width {
            break;
        }
        out.push_str(g);
        used += g.width();
    }
    out
}
fn syntax(text: &str, p: Palette) -> Vec<Span<'static>> {
    if text.starts_with('+') {
        return vec![Span::styled(text.to_owned(), Style::default().fg(p.good))];
    }
    if text.starts_with('-') {
        return vec![Span::styled(text.to_owned(), Style::default().fg(p.bad))];
    }
    let mut spans = vec![];
    let mut word = String::new();
    let flush = |word: &mut String, spans: &mut Vec<Span<'static>>| {
        if !word.is_empty() {
            let color = if [
                "pub", "fn", "let", "mut", "impl", "struct", "enum", "use", "mod", "return", "if",
                "else", "match", "async", "await", "test",
            ]
            .contains(&word.as_str())
            {
                p.build
            } else if ["bool", "i32", "u32", "u64", "String", "Self", "Result"]
                .contains(&word.as_str())
            {
                p.plan
            } else {
                p.fg
            };
            spans.push(Span::styled(
                std::mem::take(word),
                Style::default().fg(color),
            ));
        }
    };
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush(&mut word, &mut spans);
            spans.push(Span::styled(c.to_string(), Style::default().fg(p.fg)));
        }
    }
    flush(&mut word, &mut spans);
    spans
}
fn border(title: &str, right: &str, width: usize, p: Palette) -> Line<'static> {
    let title = clipped(title, width.saturating_sub(6));
    let remaining = width.saturating_sub(title.width() + 5);
    let right = clipped(right, remaining);
    let padding = width.saturating_sub(title.width() + right.width() + 5);
    Line::from(vec![
        Span::styled("┌─ ", Style::default().fg(p.border)),
        Span::styled(title, Style::default().fg(p.fg)),
        Span::styled(
            format!(" {}{}┐", "─".repeat(padding), right),
            Style::default().fg(p.border),
        ),
    ])
}
fn transcript(app: &App, width: u16, p: Palette) -> Vec<Line<'static>> {
    let mut lines = vec![];
    let width = usize::from(width).max(4);
    let start = app.transcript.entries.len().saturating_sub(256);
    if start > 0 {
        lines.push(Line::styled(
            "Histórico anterior preservado na sessão.",
            Style::default().fg(p.muted),
        ));
    }
    for entry in &app.transcript.entries[start..] {
        match entry {
            Entry::Text {
                speaker,
                text,
                mode,
            } => {
                let color = match speaker {
                    Speaker::User => p.plan,
                    Speaker::Error => p.bad,
                    Speaker::Info | Speaker::Reasoning => p.muted,
                    _ => p.fg,
                };
                if matches!(speaker, Speaker::Assistant | Speaker::Error) {
                    let edge = if *speaker == Speaker::Error {
                        p.bad
                    } else if *mode == Some(Mode::Plan) {
                        p.plan
                    } else {
                        p.build
                    };
                    let label = mode.map(mode_name).unwrap_or("RESPOSTA");
                    let title = format!("{} · {label}", speaker.label());
                    let mut frame_palette = p;
                    frame_palette.border = edge;
                    lines.push(border(&title, "", width, frame_palette));
                    let inner = width.saturating_sub(4);
                    for part in wrap(text, inner) {
                        let padding = inner.saturating_sub(part.width());
                        lines.push(Line::from(vec![
                            Span::styled("│ ", Style::default().fg(edge)),
                            Span::styled(part, Style::default().fg(color)),
                            Span::styled(
                                format!("{} │", " ".repeat(padding)),
                                Style::default().fg(edge),
                            ),
                        ]));
                    }
                    lines.push(Line::styled(
                        format!("└{}┘", "─".repeat(width.saturating_sub(2))),
                        Style::default().fg(edge),
                    ));
                    lines.push(Line::raw(""));
                    continue;
                }
                lines.push(Line::styled(
                    speaker.label(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
                lines.extend(
                    wrap(text, width)
                        .into_iter()
                        .map(|line| Line::styled(line, Style::default().fg(color))),
                );
                lines.push(Line::raw(""));
            }
            Entry::Tool { name, result, .. } => {
                let marker = match result {
                    Some(result)
                        if result.starts_with("Tool failed:") || result.starts_with("Error:") =>
                    {
                        "!"
                    }
                    Some(_) => "✓",
                    None => "·",
                };
                lines.push(Line::styled(
                    format!("{marker} {name}"),
                    Style::default().fg(p.muted),
                ));
                if let Some(result) = result {
                    let text = clipped(&result.replace('\n', " "), width.saturating_sub(2));
                    lines.push(Line::styled(text, Style::default().fg(p.muted)));
                }
                lines.push(Line::raw(""));
            }
            Entry::Code(code) => {
                lines.push(border(&code.path, code.status.label(), width, p));
                let inner = width.saturating_sub(2);
                let code_width = inner.saturating_sub(6).max(1);
                for (index, line) in code.code.split('\n').enumerate().take(1000) {
                    for (continued, part) in wrap(line, code_width).into_iter().enumerate() {
                        let number = if continued == 0 {
                            format!("{:>4} ", index + 1)
                        } else {
                            "     ".into()
                        };
                        let pad = inner.saturating_sub(number.width() + part.width());
                        let mut spans = vec![
                            Span::styled("│", Style::default().fg(p.border)),
                            Span::styled(number, Style::default().fg(p.muted)),
                        ];
                        spans.extend(syntax(&part, p));
                        spans.push(Span::styled(
                            format!("{}│", " ".repeat(pad)),
                            Style::default().fg(p.border),
                        ));
                        lines.push(Line::from(spans));
                    }
                }
                if code.truncated {
                    lines.push(Line::styled(
                        "│ Prévia limitada: 1000 linhas / 80 mil caracteres.",
                        Style::default().fg(p.build),
                    ));
                }
                lines.push(Line::styled(
                    format!("└{}┘", "─".repeat(width.saturating_sub(2))),
                    Style::default().fg(p.border),
                ));
                lines.push(Line::raw(""));
            }
        }
    }
    if lines.len() > 12000 {
        lines.drain(..lines.len() - 12000);
    }
    lines
}
/// Display only; never use this label for workspace identity or authorization.
pub fn workspace_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return if path.contains('/') { "/" } else { "\\" };
    }
    trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed)
}
fn approval_panel(frame: &mut Frame, app: &mut App, p: Palette) {
    let area = frame.area();
    let Some(approval) = &mut app.approval else {
        return;
    };
    let w = area.width.saturating_sub(4).min(100);
    let h = app
        .chat_area
        .height
        .max(10)
        .min(area.height.saturating_sub(4));
    let rect = Rect::new(area.x + (area.width - w) / 2, app.chat_area.y, w, h);
    app.approval_area = rect;
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .title(format!(" Aprovação · {} ", approval.tool))
        .borders(Borders::ALL)
        .style(p.base())
        .border_style(Style::default().fg(p.build));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let choices = if let Some(branch) = &approval.confirmation {
        format!("M confirmar envio à {branch}   N negar")
    } else if approval.allow_session {
        "Y uma vez   A nesta sessão   N negar".into()
    } else {
        "Y uma vez   N negar".into()
    };
    let mut footer = wrap(&choices, usize::from(inner.width));
    footer.extend(wrap(
        "Enter/Tab não aprovam · Esc/Ctrl+C interrompem",
        usize::from(inner.width),
    ));
    if approval.truncated {
        footer.extend(wrap("Prévia limitada: 1000 linhas / 80 mil caracteres. Negue e peça uma ação menor para revisar tudo.", usize::from(inner.width)));
    }
    footer.extend(wrap(
        "↑↓ PgUp/PgDn mouse · rolar prévia",
        usize::from(inner.width),
    ));
    let rows = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(
            footer
                .len()
                .min(usize::from(inner.height.saturating_sub(1))) as u16,
        ),
    ])
    .split(inner);
    let mut body = vec![];
    for line in approval
        .scope
        .lines()
        .chain(std::iter::once(""))
        .chain(approval.lines.iter().map(String::as_str))
    {
        for part in wrap(line, usize::from(rows[0].width)) {
            body.push(Line::from(syntax(&part, p)));
        }
    }
    approval.max_scroll = body.len().saturating_sub(usize::from(rows[0].height));
    approval.scroll = approval.scroll.min(approval.max_scroll);
    frame.render_widget(
        Paragraph::new(body).scroll((approval.scroll.min(u16::MAX as usize) as u16, 0)),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(footer.join("\n")).style(Style::default().fg(p.build)),
        rows[1],
    );
}
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let p = app.theme.palette(app.no_color);
    frame.render_widget(Block::default().style(p.base()), area);
    if area.width < 45 || area.height < 13 {
        frame.render_widget(
            Paragraph::new(
                "My-Agent\nAmplie o terminal (mínimo 45 × 13).\nEsc/Ctrl+C continuam disponíveis.",
            )
            .style(p.base())
            .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(5),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    let active = if app.mode == Mode::Plan {
        p.plan
    } else {
        p.build
    };
    let header = Line::from(vec![
        Span::styled("My-Agent", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(
            format!(
                "  │  {}  │  {}  │  ",
                crate::safe_text(workspace_name(&app.workspace)),
                crate::safe_text(&app.model)
            ),
            Style::default().fg(p.muted),
        ),
        Span::styled(
            if app.mode == Mode::Plan {
                "[PLAN]"
            } else {
                " PLAN "
            },
            Style::default().fg(p.plan),
        ),
        Span::raw("  "),
        Span::styled(
            if app.mode == Mode::Build {
                "[BUILD]"
            } else {
                " BUILD "
            },
            Style::default().fg(p.build),
        ),
    ]);
    // Reserve room for the selector even when the workspace path is long.
    let header_parts =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(19)]).split(layout[0]);
    let left = header.spans[..2].to_vec();
    frame.render_widget(
        Paragraph::new(Line::from(left)).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(p.border)),
        ),
        header_parts[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(header.spans[2..].to_vec())).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(p.border)),
        ),
        header_parts[1],
    );
    app.chat_area = layout[1];
    app.approval_area = Rect::default();
    if app.transcript.entries.is_empty() {
        let title = if app.screen == Screen::NewSession {
            "Nova sessão"
        } else {
            "O que vamos construir?"
        };
        let home = Rect::new(
            layout[1].x,
            layout[1].y + layout[1].height.saturating_sub(5) / 2,
            layout[1].width,
            layout[1].height.min(5),
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(
                    title,
                    Style::default().fg(p.fg).add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    "PLAN para explorar · BUILD para implementar",
                    Style::default().fg(p.muted),
                ),
                Line::styled(
                    "Descreva a tarefa ou use / para comandos.",
                    Style::default().fg(p.muted),
                ),
            ])
            .alignment(ratatui::layout::Alignment::Center),
            home,
        );
    } else {
        let lines = transcript(app, layout[1].width, p);
        app.max_scroll = lines.len().saturating_sub(usize::from(layout[1].height));
        if app.follow {
            app.scroll = app.max_scroll;
        }
        app.scroll = app.scroll.min(app.max_scroll);
        frame.render_widget(
            Paragraph::new(lines).scroll((app.scroll.min(u16::MAX as usize) as u16, 0)),
            layout[1],
        );
    }
    let block = Block::default()
        .title(" Mensagem ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border));
    let mut inner = block.inner(layout[2]);
    frame.render_widget(block, layout[2]);
    frame.render_widget(
        Paragraph::new(
            (0..layout[2].height)
                .map(|_| "┃")
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .style(Style::default().fg(active)),
        Rect::new(layout[2].x, layout[2].y, 1, layout[2].height),
    );
    inner.x = inner.x.saturating_add(1);
    inner.width = inner.width.saturating_sub(1);
    let (input_lines, (cursor_col, cursor_row)) = app.input.visual(usize::from(inner.width));
    let scroll = cursor_row.saturating_sub(usize::from(inner.height).saturating_sub(1));
    let text = if app.input.text().is_empty() {
        vec![Line::styled(
            "Digite sua mensagem para o My-Agent...",
            Style::default().fg(p.muted),
        )]
    } else {
        input_lines.into_iter().map(Line::raw).collect()
    };
    frame.render_widget(Paragraph::new(text).scroll((scroll as u16, 0)), inner);
    if app.dialog.is_none() && app.approval.is_none() && !app.busy() {
        frame.set_cursor_position((
            inner.x + cursor_col.min(usize::from(inner.width.saturating_sub(1))) as u16,
            inner.y
                + cursor_row
                    .saturating_sub(scroll)
                    .min(usize::from(inner.height.saturating_sub(1))) as u16,
        ));
    }
    let token = Layout::horizontal([Constraint::Length(32), Constraint::Min(4)]).split(Rect::new(
        layout[3].x,
        layout[3].y,
        layout[3].width,
        1,
    ));
    let label = if app.measured.is_some() || app.live_tokens.is_some() {
        format!(
            "Tokens: {} / 10M · {:.0}%",
            format_tokens(app.tokens()),
            token_ratio(app.tokens()) * 100.0
        )
    } else {
        "Tokens: sem medição / 10M".into()
    };
    frame.render_widget(
        Paragraph::new(label).style(Style::default().fg(p.muted)),
        token[0],
    );
    frame.render_widget(
        Gauge::default()
            .ratio(token_ratio(app.tokens()))
            .label("")
            .gauge_style(Style::default().fg(active).bg(p.border)),
        token[1],
    );
    let spinner = if app.busy() {
        ["⠋", "⠙", "⠹", "⠸"][((app.tick / 3) % 4) as usize]
    } else {
        "·"
    };
    let running = app
        .running_mode
        .map(|mode| format!(" · {} em execução", mode_name(mode)))
        .unwrap_or_default();
    let status = format!(
        "{spinner} {}{running} · {} sem medição",
        app.status, app.unreported
    );
    if let Some(toast) = &app.toast {
        frame.render_widget(
            Paragraph::new(toast.text.clone()).style(Style::default().fg(if toast.error {
                p.bad
            } else {
                p.good
            })),
            layout[4],
        );
    } else {
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(p.muted)),
            layout[4],
        );
    }
    frame.render_widget(
        Paragraph::new(
            "Enter enviar · Ctrl+J nova linha · Tab modo · Esc/Ctrl+C interromper · / comandos",
        )
        .style(Style::default().fg(p.muted)),
        layout[5],
    );
    approval_panel(frame, app, p);
    if let Some(dialog) = &app.dialog {
        let w = area.width.saturating_sub(8).min(88);
        let h = area.height.saturating_sub(6).min(19);
        let rect = Rect::new((area.width - w) / 2, (area.height - h) / 2, w, h);
        frame.render_widget(Clear, rect);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL)
            .style(p.base())
            .border_style(Style::default().fg(p.build));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let rows = Layout::vertical([
            Constraint::Length(
                if dialog.kind == crate::dialog::DialogKind::ChatGpt && !dialog.choices.is_empty() {
                    4
                } else {
                    2
                },
            ),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
        frame.render_widget(
            Paragraph::new(
                if dialog.kind == crate::dialog::DialogKind::ChatGpt && !dialog.choices.is_empty() {
                    format!(
                        "{}\nBuscar: {}",
                        dialog.note,
                        crate::safe_text(&dialog.query)
                    )
                } else {
                    format!("Buscar: {}", crate::safe_text(&dialog.query))
                },
            )
            .style(Style::default().fg(p.muted))
            .wrap(Wrap { trim: false }),
            rows[0],
        );
        let filtered = dialog.filtered();
        let visible = usize::from(rows[1].height).max(1);
        let start = dialog.selected.saturating_sub(visible - 1);
        let mut lines = vec![];
        if filtered.is_empty() {
            lines.extend(
                dialog
                    .note
                    .lines()
                    .flat_map(|line| wrap(line, usize::from(rows[1].width)))
                    .map(Line::raw),
            );
        }
        for (index, choice) in filtered.iter().enumerate().skip(start).take(visible) {
            let selected = index == dialog.selected;
            lines.push(Line::styled(
                format!(
                    "{} {}",
                    if selected { "›" } else { " " },
                    crate::safe_text(&choice.label)
                ),
                if selected {
                    Style::default().fg(p.build).add_modifier(Modifier::BOLD)
                } else {
                    p.base()
                },
            ));
        }
        frame.render_widget(Paragraph::new(lines), rows[1]);
        frame.render_widget(
            Paragraph::new("↑↓ selecionar · Enter confirmar · Esc voltar")
                .style(Style::default().fg(p.muted)),
            rows[2],
        );
    } else if app.approval.is_none() && !app.busy() {
        let items: Vec<String> = if app.command_menu_open() {
            app.commands()
                .iter()
                .enumerate()
                .map(|(i, (name, help))| {
                    format!(
                        "{} {name}  {help}",
                        if i == app.menu_selected { "›" } else { " " }
                    )
                })
                .collect()
        } else {
            app.mentions.iter().map(|path| format!("@{path}")).collect()
        };
        if !items.is_empty() {
            let h = ((items.len() + 2).min(10) as u16).min(app.chat_area.height);
            if h < 3 {
                return;
            }
            let visible = usize::from(h - 2);
            let start = app.menu_selected.saturating_sub(visible - 1);
            let rect = Rect::new(
                layout[2].x,
                layout[2].y.saturating_sub(h),
                layout[2].width.min(70),
                h,
            );
            frame.render_widget(Clear, rect);
            frame.render_widget(
                Paragraph::new(
                    items
                        .into_iter()
                        .skip(start)
                        .take(visible)
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
                .block(
                    Block::default()
                        .title(" Tab completar · Esc fechar ")
                        .borders(Borders::ALL),
                )
                .style(p.base()),
                rect,
            );
        }
    }
}
