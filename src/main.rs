use std::io::Write;
use std::fs::{File, OpenOptions};
use std::io;
use std::io::Stdout;
use crate::telem::GpuTelemetry;
use anyhow::{Result};
use tokio::time::{interval, Duration};
use crossterm::terminal::{
    enable_raw_mode, disable_raw_mode,
    EnterAlternateScreen, LeaveAlternateScreen
};
use crossterm::execute;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};

mod telem;

const DRI: &str = "/sys/kernel/debug/dri/0";

#[tokio::main]
async fn main() -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/amdtelem.log")?;

    let result = run(&mut terminal, log).await;

    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;

    result
}

async fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, mut log: File) -> Result<()> {
    let mut telem = GpuTelemetry::init()?;
    let mut ticker = interval(Duration::from_secs(1));

    loop {
        if event::poll(Duration::from_millis(0))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                    return Ok(())
                }
            }
        }

        ticker.tick().await;

        telem.data.last_errors.clear();

        if let Err(e) = telem.collect_hwmon() {
            telem.data.last_errors.push(format!("[hwmon error] {}", e));
            writeln!(log, "[hwmon error] {}", e)?;
        }

        if let Err(e) = telem.collect_pm_info() {
            telem.data.last_errors.push(format!("[pm error] {}", e));
            writeln!(log, "[pm error] {}", e)?;
        }

        if let Err(e) = telem.collect_gem_info() {
            telem.data.last_errors.push(format!("[gem error] {}", e));
            writeln!(log, "[gem error] {}", e)?;
        }

        if let Err(e) = telem.collect_mclk() {
            telem.data.last_errors.push(format!("[mclk error] {}", e));
            writeln!(log, "[mclk error] {}", e)?;
        }

        let junc_text = telem.data.junction_temp_c
            .map_or("N/A".to_string(), |v| format!("{:.1}°C", v));

        let junc_color = telem.data.junction_temp_c
            .map_or(Color::Gray, |v| temp_color(v));

        let mem_text = telem.data.memory_temp_c
            .map_or("N/A".to_string(), |v| format!("{:.1}°C", v));

        let mem_color = telem.data.memory_temp_c
            .map_or(Color::Gray, |v| temp_color(v));

        let vddnb_text = telem.data.vddnb_mv
            .map_or("N/A".to_string(), |v| format!("{} mV", v));

        let fan_text = telem.data.fan_rpm
            .map_or("N/A".to_string(), |v| format!("{} RPM", v));


        terminal.draw(|frame| {
            let area = frame.area();

            let outer_block = Block::default()
                .borders(Borders::ALL)
                .title(
                    ratatui::widgets::block::Title::from(
                        Span::styled("AMD Radeon Telemetry", Style::default().fg(Color::Cyan))
                    )
                );

            let inner_area = outer_block.inner(area);
            frame.render_widget(outer_block, area);

            let error_height = if telem.data.last_errors.is_empty() { 0 } else { 3 };

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(7),
                    Constraint::Min(4),
                    Constraint::Length(error_height),
                ])
                .split(inner_area);

            let stats_text = Text::from(vec![
                Line::from(vec![
                    Span::styled("GPU           | ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        format!("{}", telem.get_gpu_name()),
                        Style::default().fg(Color::LightRed),
                    )
                ]),
                Line::from(vec![
                    Span::styled("Temperatures  | ", Style::default().fg(Color::Gray)),
                    Span::styled("Edge:  ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        format!("{:3.1}°C", telem.data.edge_temp_c),
                        Style::default().fg(temp_color(telem.data.edge_temp_c)),
                    ),
                    Span::raw("      "),
                    Span::styled("Junc:  ", Style::default().fg(Color::Gray)),
                    Span::styled(junc_text, Style::default().fg(junc_color)),

                    Span::raw("      "),
                    Span::styled("Mem: ", Style::default().fg(Color::Gray)),
                    Span::styled(mem_text, Style::default().fg(mem_color)),
                ]),
                Line::from(vec![
                    Span::styled("Clocks        | ", Style::default().fg(Color::Gray)),
                    Span::styled("SCLK: ", Style::default().fg(Color::Gray)),
                    Span::styled(format!("{:3} MHz", telem.data.sclk_mhz), Style::default().fg(Color::Gray)),
                    Span::raw("      "),
                    Span::styled("MCLK: ", Style::default().fg(Color::Gray)),
                    Span::styled(format!("{:3} MHz", telem.data.mclk_mhz), Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled("Power         | ", Style::default().fg(Color::Gray)),
                    Span::styled("Avg:   ", Style::default().fg(Color::Gray)),
                    Span::styled(format!("{:3.2}W", telem.data.power_avg_w), Style::default().fg(Color::Gray)),
                    Span::raw("      "),
                    Span::styled("SoC:   ", Style::default().fg(Color::Gray)),
                    Span::styled(format!("{:3.2}W", telem.data.soc_wattage), Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                    Span::styled("Load          | ", Style::default().fg(Color::Gray)),
                    Span::styled("GPU:    ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        format!("{}%", telem.data.gpu_load_pct),
                        Style::default().fg(load_color(telem.data.gpu_load_pct)),
                    ),
                    Span::raw("         "),
                    Span::styled("VCN:    ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        format!("{}%", telem.data.vcn_load_pct),
                        Style::default().fg(load_color(telem.data.vcn_load_pct)),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Voltage       | ", Style::default().fg(Color::Gray)),
                    Span::styled("VDDGFX: ", Style::default().fg(Color::Gray)),
                    Span::styled(format!("{} mV", telem.data.vddgfx_mv), Style::default().fg(Color::Gray)),
                    Span::raw("     "),
                    Span::styled("VDDNB: ", Style::default().fg(Color::Gray)),
                    Span::styled(vddnb_text, Style::default().fg(Color::Gray)),
                ]),
                Line::from(vec![
                   Span::styled("Fan           | RPM:", Style::default().fg(Color::Gray)),
                   Span::raw("    "),
                   Span::styled(fan_text, Style::default().fg(Color::Gray)),
                ]),

            ]);

            let stats = Paragraph::new(stats_text);
            frame.render_widget(stats, chunks[0]);

            let gem_block = Block::default()
                .borders(Borders::TOP)
                .title(
                    ratatui::widgets::block::Title::from(
                        Span::styled("GEM Clients", Style::default().fg(Color::Cyan))
                    )
                );

            let mut gem_spans: Vec<Span> = Vec::new();
            for (i, client) in telem.data.gem_clients.iter().enumerate() {
                if i > 0 {
                    gem_spans.push(Span::raw("  "));
                }
                gem_spans.push(Span::styled(
                    client.command.clone(),
                    Style::default().fg(Color::Gray),
                ));
                gem_spans.push(Span::styled(
                    format!(" ({})", client.pid),
                    Style::default().fg(Color::Gray),
                ));
            }

            let gem_text = Text::from(vec![Line::from(gem_spans)]);

            let gem_paragraph = Paragraph::new(gem_text).block(gem_block);
            frame.render_widget(gem_paragraph, chunks[1]);

            if !telem.data.last_errors.is_empty() {
                let error_block = Block::default()
                    .borders(Borders::TOP)
                    .title(
                        ratatui::widgets::block::Title::from(
                            Span::styled("Errors", Style::default().fg(Color::Red))
                        )
                    );

                let error_text = Text::from(
                    telem.data.last_errors
                        .iter()
                        .map(|e| Line::from(
                            Span::styled(e.clone(), Style::default().fg(Color::Red))
                        ))
                        .collect::<Vec<_>>()
                );

                let error_paragraph = Paragraph::new(error_text).block(error_block);
                frame.render_widget(error_paragraph, chunks[2]);
            }

        })?;

    }
}

fn temp_color(temp: f64) -> Color {
    match temp as u64 {
        0..=60  => Color::Green,
        61..=85 => Color::Yellow,
        _       => Color::Red,
    }
}

fn load_color(pct: u64) -> Color {
    match pct {
        0..=40  => Color::Green,
        41..=80 => Color::Yellow,
        _       => Color::Red,
    }
}
