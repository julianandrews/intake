use crate::config::Column;
use chrono::{NaiveTime, Timelike};
use std::fmt::Write;

pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_CYAN: &str = "\x1b[36m";
pub const ANSI_BOLD_BLUE: &str = "\x1b[1;34m";
pub const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
pub const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
pub const ANSI_BOLD_RED: &str = "\x1b[1;31m";
pub const ANSI_BOLD_MAGENTA: &str = "\x1b[1;35m";
pub const ANSI_DIM: &str = "\x1b[2m";
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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ColumnTarget {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

pub trait ColumnValue {
    fn column_value(&self, column: Column) -> f64;
}

macro_rules! impl_column_value {
    ($t:ty, $calories:ident, $protein:ident, $fiber:ident, $fat:ident, $carbs:ident, $alcohol:ident) => {
        impl $crate::display::ColumnValue for $t {
            fn column_value(&self, column: $crate::config::Column) -> f64 {
                match column {
                    $crate::config::Column::Calories => f64::from(self.$calories),
                    $crate::config::Column::Protein => self.$protein,
                    $crate::config::Column::Fiber => self.$fiber,
                    $crate::config::Column::Fat => self.$fat,
                    $crate::config::Column::Carbs => self.$carbs,
                    $crate::config::Column::Alcohol => self.$alcohol,
                }
            }
        }
    };
}
pub(crate) use impl_column_value;

#[derive(Debug, Clone, Copy, Default)]
pub struct DayTotals {
    pub calories: f64,
    pub protein: f64,
    pub fiber: f64,
    pub fat: f64,
    pub carbs: f64,
    pub alcohol: f64,
}

impl_column_value!(DayTotals, calories, protein, fiber, fat, carbs, alcohol);

#[derive(Debug, Clone, Copy, Default)]
pub struct DayTargets {
    pub calories: ColumnTarget,
    pub protein: ColumnTarget,
    pub fiber: ColumnTarget,
    pub fat: ColumnTarget,
    pub carbs: ColumnTarget,
    pub alcohol: ColumnTarget,
}

impl DayTargets {
    pub fn for_column(&self, column: Column) -> ColumnTarget {
        match column {
            Column::Calories => self.calories,
            Column::Protein => self.protein,
            Column::Fiber => self.fiber,
            Column::Fat => self.fat,
            Column::Carbs => self.carbs,
            Column::Alcohol => self.alcohol,
        }
    }
}

pub fn column_color(
    now: Option<NaiveTime>,
    value: f64,
    target: &ColumnTarget,
) -> Option<&'static str> {
    let dp = now.as_ref().map(day_proportion).unwrap_or(1.0);

    if let Some(max) = target.max {
        if value > max {
            return Some(ANSI_RED);
        }
        if value > max * dp {
            return Some(ANSI_YELLOW);
        }
    }
    if let Some(min) = target.min {
        if value < min * dp {
            return Some(ANSI_YELLOW);
        }
    }
    (target.min.is_some() || target.max.is_some()).then_some(ANSI_GREEN)
}

pub fn log_cell(column: Column, value: f64) -> String {
    match column {
        Column::Calories => format!("{value:.0}"),
        _ => format!("{value:.1}"),
    }
}

pub fn food_cell(column: Column, value: f64) -> String {
    match column {
        Column::Calories => format!("{}", value.round() as u32),
        _ => format!("{value:.1}g"),
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
            calories: ColumnTarget {
                min: None,
                max: Some(2000.0),
            },
            protein: ColumnTarget {
                min: Some(100.0),
                max: None,
            },
            fiber: ColumnTarget {
                min: Some(20.0),
                max: None,
            },
            ..DayTargets::default()
        }
    }

    #[test]
    fn test_column_color_past_day() {
        assert_eq!(
            column_color(None, 2100.0, &targets().calories),
            Some(ANSI_RED)
        );
        assert_eq!(
            column_color(None, 90.0, &targets().protein),
            Some(ANSI_YELLOW)
        );
        assert_eq!(column_color(None, 25.0, &targets().fiber), Some(ANSI_GREEN));
    }

    #[test]
    fn test_column_color_today_at_noon() {
        let now = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        let calories = targets().calories;

        // net 800: at/below half of 2000 -> green
        assert_eq!(column_color(Some(now), 800.0, &calories), Some(ANSI_GREEN));

        // net 1200: over half, under target -> yellow
        assert_eq!(
            column_color(Some(now), 1200.0, &calories),
            Some(ANSI_YELLOW)
        );

        // net 2500: over target -> red
        assert_eq!(column_color(Some(now), 2500.0, &calories), Some(ANSI_RED));

        // protein 40 >= 50? no -> yellow; fiber 5 >= 10? no -> yellow
        assert_eq!(
            column_color(Some(now), 40.0, &targets().protein),
            Some(ANSI_YELLOW)
        );
        assert_eq!(
            column_color(Some(now), 5.0, &targets().fiber),
            Some(ANSI_YELLOW)
        );
    }

    #[test]
    fn test_column_color_min_and_max_band() {
        let target = ColumnTarget {
            min: Some(50.0),
            max: Some(90.0),
        };
        // below min -> yellow
        assert_eq!(column_color(None, 40.0, &target), Some(ANSI_YELLOW));
        // at min -> green
        assert_eq!(column_color(None, 50.0, &target), Some(ANSI_GREEN));
        // in band -> green
        assert_eq!(column_color(None, 70.0, &target), Some(ANSI_GREEN));
        // above max -> red
        assert_eq!(column_color(None, 95.0, &target), Some(ANSI_RED));
    }

    #[test]
    fn test_column_color_band_scales_with_day_progress() {
        let target = ColumnTarget {
            min: Some(50.0),
            max: Some(90.0),
        };
        let now = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        // band halves at noon: 25..45
        assert_eq!(column_color(Some(now), 20.0, &target), Some(ANSI_YELLOW));
        assert_eq!(column_color(Some(now), 30.0, &target), Some(ANSI_GREEN));
        // between scaled max (45) and full max (90) -> yellow
        assert_eq!(column_color(Some(now), 50.0, &target), Some(ANSI_YELLOW));
        // above full max -> red
        assert_eq!(column_color(Some(now), 95.0, &target), Some(ANSI_RED));
    }

    #[test]
    fn test_column_color_no_targets() {
        let target = ColumnTarget::default();
        assert_eq!(column_color(None, 2100.0, &target), None);
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
    fn test_column_color_past_day_below_targets() {
        assert_eq!(
            column_color(None, 1500.0, &targets().calories),
            Some(ANSI_GREEN)
        );
        assert_eq!(
            column_color(None, 50.0, &targets().protein),
            Some(ANSI_YELLOW)
        );
        assert_eq!(
            column_color(None, 10.0, &targets().fiber),
            Some(ANSI_YELLOW)
        );
    }

    #[test]
    fn test_log_cell_formats() {
        assert_eq!(log_cell(Column::Calories, 1500.4), "1500");
        assert_eq!(log_cell(Column::Protein, 12.34), "12.3");
        assert_eq!(log_cell(Column::Fat, 7.0), "7.0");
        assert_eq!(log_cell(Column::Alcohol, 2.5), "2.5");
    }

    #[test]
    fn test_food_cell_formats() {
        assert_eq!(food_cell(Column::Calories, 160.0), "160");
        assert_eq!(food_cell(Column::Protein, 9.0), "9.0g");
        assert_eq!(food_cell(Column::Carbs, 30.25), "30.2g");
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
