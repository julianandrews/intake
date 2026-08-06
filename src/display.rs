use chrono::{NaiveTime, Timelike};
use std::fmt::Write;

pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_CYAN: &str = "\x1b[36m";
pub const ANSI_BOLD_BLUE: &str = "\x1b[1;34m";
pub const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
pub const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
pub const ANSI_BOLD_RED: &str = "\x1b[1;31m";
pub const ANSI_BOLD_MAGENTA: &str = "\x1b[1;35m";
pub const ANSI_GREEN: &str = "\x1b[32m";
pub const ANSI_YELLOW: &str = "\x1b[33m";
pub const ANSI_RED: &str = "\x1b[31m";

#[derive(Debug, Clone)]
pub enum Align {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
enum FooterStyle {
    Alternating,
    Custom,
}

#[derive(Debug)]
struct FooterRow {
    cells: Vec<String>,
    style: FooterStyle,
}

#[derive(Debug)]
pub struct Table {
    headers: Vec<String>,
    align: Vec<Align>,
    rows: Vec<Vec<String>>,
    title: Option<String>,
    footers: Vec<FooterRow>,
}

pub fn visible_width(s: &str) -> usize {
    let mut len = 0;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            for n in chars.by_ref() {
                if n == 'm' {
                    break;
                }
            }
        } else {
            len += 1;
        }
    }
    len
}

fn format_cells(cells: &[String], widths: &[usize], align: &[Align]) -> String {
    let mut line = String::new();
    for (i, (cell, width)) in cells.iter().zip(widths).enumerate() {
        let vis = visible_width(cell);
        let pad = width.saturating_sub(vis);
        match align[i] {
            Align::Left => {
                write!(line, "{}", cell).unwrap();
                for _ in 0..pad {
                    line.push(' ');
                }
            }
            Align::Right => {
                for _ in 0..pad {
                    line.push(' ');
                }
                write!(line, "{}", cell).unwrap();
            }
        }
        if i == 0 {
            line.push(' ');
        } else if i < cells.len() - 1 {
            line.push_str("  ");
        }
    }
    line
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        let n = headers.len();
        let mut align = vec![Align::Right; n];
        if n > 0 {
            align[0] = Align::Left;
        }
        Table {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            align,
            rows: Vec::new(),
            title: None,
            footers: Vec::new(),
        }
    }

    pub fn with_align(headers: &[&str], align: &[Align]) -> Self {
        Table {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            align: align.to_vec(),
            rows: Vec::new(),
            title: None,
            footers: Vec::new(),
        }
    }

    pub fn add_row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn set_title(&mut self, title: &str) {
        self.title = Some(title.to_string());
    }

    pub fn add_footer(&mut self, row: Vec<String>) {
        self.footers.push(FooterRow {
            cells: row,
            style: FooterStyle::Alternating,
        });
    }

    pub fn add_footer_custom(&mut self, row: Vec<String>) {
        self.footers.push(FooterRow {
            cells: row,
            style: FooterStyle::Custom,
        });
    }

    fn col_widths(&self) -> Vec<usize> {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| visible_width(h)).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(visible_width(cell));
            }
        }
        for footer in &self.footers {
            for (i, cell) in footer.cells.iter().enumerate() {
                widths[i] = widths[i].max(visible_width(cell));
            }
        }
        widths
    }

    pub fn format(&self) -> String {
        let widths = self.col_widths();
        let n = widths.len();
        let sep_total = if n <= 1 { 0 } else { 1 + 2 * (n - 2) };
        let sep_width = 2 + widths.iter().sum::<usize>() + sep_total;

        let mut out = String::new();

        if let Some(title) = &self.title {
            writeln!(out, "{ANSI_BOLD_CYAN}{title}{ANSI_RESET}").unwrap();
        }

        writeln!(out, "{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width)).unwrap();

        writeln!(
            out,
            "  {ANSI_BOLD_YELLOW}{}{ANSI_RESET}",
            format_cells(&self.headers, &widths, &self.align)
        )
        .unwrap();

        let dash_cells: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
        writeln!(
            out,
            "  {ANSI_CYAN}{}{ANSI_RESET}",
            format_cells(&dash_cells, &widths, &self.align)
        )
        .unwrap();

        for row in &self.rows {
            writeln!(out, "  {}", format_cells(row, &widths, &self.align)).unwrap();
        }

        writeln!(out, "{ANSI_CYAN}{}{ANSI_RESET}", "-".repeat(sep_width)).unwrap();

        let mut colored_idx = 0;
        for footer in &self.footers {
            match footer.style {
                FooterStyle::Alternating => {
                    let ansi = if colored_idx % 2 == 0 {
                        ANSI_BOLD_MAGENTA
                    } else {
                        ANSI_BOLD_BLUE
                    };
                    writeln!(
                        out,
                        "  {ansi}{}{ANSI_RESET}",
                        format_cells(&footer.cells, &widths, &self.align)
                    )
                    .unwrap();
                    colored_idx += 1;
                }
                FooterStyle::Custom => {
                    writeln!(
                        out,
                        "  {}",
                        format_cells(&footer.cells, &widths, &self.align)
                    )
                    .unwrap();
                }
            }
        }

        out
    }
}

fn day_proportion(now: &NaiveTime) -> f64 {
    let elapsed = now.hour() * 3600 + now.minute() * 60 + now.second();
    elapsed as f64 / 86400.0
}

pub fn wrap_color(value: &str, color: Option<&str>) -> String {
    match color {
        Some(color) => format!("{color}{value}{ANSI_RESET}"),
        None => value.to_string(),
    }
}

pub struct DayTotals {
    pub protein: f64,
    pub fiber: f64,
}

pub struct DayTargets {
    pub max_calories: Option<u32>,
    pub min_protein: Option<f64>,
    pub min_fiber: Option<f64>,
}

pub struct TotalColors {
    pub calories: Option<&'static str>,
    pub protein: Option<&'static str>,
    pub fiber: Option<&'static str>,
}

pub fn total_cell_colors(
    now: Option<NaiveTime>,
    net_calories: f64,
    totals: &DayTotals,
    targets: &DayTargets,
) -> TotalColors {
    let dp = now.as_ref().map(day_proportion).unwrap_or(1.0);

    let calories = targets.max_calories.map(|target| {
        let target = target as f64;
        if net_calories <= target * dp {
            ANSI_GREEN
        } else if net_calories <= target {
            ANSI_YELLOW
        } else {
            ANSI_RED
        }
    });

    let protein = targets.min_protein.map(|target| {
        if totals.protein >= target * dp {
            ANSI_GREEN
        } else {
            ANSI_YELLOW
        }
    });

    let fiber = targets.min_fiber.map(|target| {
        if totals.fiber >= target * dp {
            ANSI_GREEN
        } else {
            ANSI_YELLOW
        }
    });

    TotalColors {
        calories,
        protein,
        fiber,
    }
}

pub fn render_day_summary(
    exercise_calories: u32,
    maintenance_calories: Option<u32>,
    deficit: Option<f64>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(mc) = maintenance_calories {
        let tdee = mc + exercise_calories;
        parts.push(format!("TDEE: {tdee}"));
    }
    if let Some(d) = deficit {
        let color = if d >= 0.0 { ANSI_GREEN } else { ANSI_RED };
        parts.push(format!("Deficit: {color}{d:.0}{ANSI_RESET}"));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("  {}\n", parts.join("    "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_basic() {
        let mut table = Table::with_align(
            &["Ingredient", "Amount", "Calories", "Protein(g)", "Fiber(g)"],
            &[
                Align::Left,
                Align::Left,
                Align::Right,
                Align::Right,
                Align::Right,
            ],
        );
        table.set_title("Oatmeal (2 servings)");
        table.add_row(vec![
            "Oats".to_string(),
            "100g".to_string(),
            "200".to_string(),
            "10.0g".to_string(),
            "5.0g".to_string(),
        ]);
        table.add_row(vec![
            "Milk".to_string(),
            "200ml".to_string(),
            "120".to_string(),
            "8.0g".to_string(),
            "0.0g".to_string(),
        ]);
        table.add_footer(vec![
            "Total".to_string(),
            "".to_string(),
            "320".to_string(),
            "18.0g".to_string(),
            "5.0g".to_string(),
        ]);
        table.add_footer(vec![
            "Per serving".to_string(),
            "".to_string(),
            "160".to_string(),
            "9.0g".to_string(),
            "2.5g".to_string(),
        ]);

        let md = table.format();
        assert!(md.starts_with("\u{1b}[1;36mOatmeal (2 servings)\u{1b}[0m\n"));
        assert!(md.contains("  Oats        100g         200       10.0g"));
        assert!(md.contains("  Milk        200ml        120        8.0g"));
        assert!(md.contains("----------- ------  --------  ----------  --------"));
        assert!(md.contains("\u{1b}[1;35mTotal                    320       18.0g"));
        assert!(md.contains("\u{1b}[1;34mPer serving              160        9.0g"));
    }

    #[test]
    fn test_display_single_serving() {
        let mut table = Table::new(&["Ingredient", "Amount", "Calories", "Protein(g)", "Fiber(g)"]);
        table.set_title("Coffee (1 serving)");
        table.add_row(vec![
            "Cold Brew".to_string(),
            "-".to_string(),
            "0".to_string(),
            "0.0g".to_string(),
            "0.0g".to_string(),
        ]);

        let md = table.format();
        assert!(md.starts_with("\u{1b}[1;36mCoffee (1 serving)\u{1b}[0m\n"));
        assert!(md.contains("  Cold Brew"));
    }

    #[test]
    fn test_visible_width_no_ansi() {
        assert_eq!(visible_width("hello"), 5);
    }

    fn targets() -> DayTargets {
        DayTargets {
            max_calories: Some(2000),
            min_protein: Some(100.0),
            min_fiber: Some(20.0),
        }
    }

    #[test]
    fn test_total_cell_colors_past_day() {
        let totals = DayTotals {
            protein: 90.0,
            fiber: 25.0,
        };
        let colors = total_cell_colors(None, 2100.0, &totals, &targets());
        assert_eq!(colors.calories, Some(ANSI_RED));
        assert_eq!(colors.protein, Some(ANSI_YELLOW));
        assert_eq!(colors.fiber, Some(ANSI_GREEN));
    }

    #[test]
    fn test_total_cell_colors_today_at_noon() {
        let now = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        let totals = DayTotals {
            protein: 40.0,
            fiber: 5.0,
        };

        // net 800: at/below half of 2000 -> green
        let on_track = total_cell_colors(Some(now), 800.0, &totals, &targets());
        assert_eq!(on_track.calories, Some(ANSI_GREEN));

        // net 1200: over half, under target -> yellow
        let over_proportion = total_cell_colors(Some(now), 1200.0, &totals, &targets());
        assert_eq!(over_proportion.calories, Some(ANSI_YELLOW));

        // net 2500: over target -> red
        let over_target = total_cell_colors(Some(now), 2500.0, &totals, &targets());
        assert_eq!(over_target.calories, Some(ANSI_RED));

        // protein 40 >= 50? no -> yellow; fiber 5 >= 10? no -> yellow
        assert_eq!(over_target.protein, Some(ANSI_YELLOW));
        assert_eq!(over_target.fiber, Some(ANSI_YELLOW));
    }

    #[test]
    fn test_total_cell_colors_no_targets() {
        let targets = DayTargets {
            max_calories: None,
            min_protein: None,
            min_fiber: None,
        };
        let colors = total_cell_colors(
            None,
            2100.0,
            &DayTotals {
                protein: 1.0,
                fiber: 1.0,
            },
            &targets,
        );
        assert_eq!(colors.calories, None);
        assert_eq!(colors.protein, None);
        assert_eq!(colors.fiber, None);
    }

    #[test]
    fn test_render_day_summary() {
        let out = render_day_summary(300, Some(2400), Some(1500.0));
        assert!(out.contains("TDEE: 2700"));
        assert!(out.contains("Deficit: \u{1b}[32m1500\u{1b}[0m"));
    }

    #[test]
    fn test_render_day_summary_empty() {
        assert_eq!(render_day_summary(0, None, None), "");
    }

    #[test]
    fn test_render_day_summary_negative_deficit() {
        let out = render_day_summary(0, Some(2400), Some(-500.0));
        assert!(out.contains("Deficit: \u{1b}[31m-500\u{1b}[0m"));
    }

    #[test]
    fn test_total_cell_colors_past_day_below_targets() {
        let totals = DayTotals {
            protein: 50.0,
            fiber: 10.0,
        };
        let colors = total_cell_colors(None, 1500.0, &totals, &targets());
        assert_eq!(colors.calories, Some(ANSI_GREEN));
        assert_eq!(colors.protein, Some(ANSI_YELLOW));
        assert_eq!(colors.fiber, Some(ANSI_YELLOW));
    }

    #[test]
    fn test_visible_width_with_ansi() {
        assert_eq!(visible_width("\x1b[1;32mhello\x1b[0m"), 5);
    }

    #[test]
    fn test_visible_width_empty() {
        assert_eq!(visible_width(""), 0);
    }
}
