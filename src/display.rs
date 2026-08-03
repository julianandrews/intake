use chrono::{NaiveTime, Timelike};
use std::fmt::Write;

pub const ANSI_RESET: &str = "\x1b[0m";
pub const ANSI_CYAN: &str = "\x1b[36m";
pub const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
pub const ANSI_BOLD_GREEN: &str = "\x1b[1;32m";
pub const ANSI_BOLD_YELLOW: &str = "\x1b[1;33m";
pub const ANSI_BOLD_RED: &str = "\x1b[1;31m";

#[derive(Debug, Clone)]
pub enum Align {
    Left,
    Right,
}

#[derive(Debug)]
pub struct Table {
    headers: Vec<String>,
    align: Vec<Align>,
    rows: Vec<Vec<String>>,
    title: Option<String>,
    footers: Vec<Vec<String>>,
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
        self.footers.push(row);
    }

    fn col_widths(&self) -> Vec<usize> {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| visible_width(h)).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(visible_width(cell));
            }
        }
        for row in &self.footers {
            for (i, cell) in row.iter().enumerate() {
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

        for (i, row) in self.footers.iter().enumerate() {
            let ansi = if i == 0 {
                ANSI_BOLD_GREEN
            } else {
                ANSI_BOLD_CYAN
            };
            writeln!(
                out,
                "  {ansi}{}{ANSI_RESET}",
                format_cells(row, &widths, &self.align)
            )
            .unwrap();
        }

        out
    }
}

fn day_proportion(now: &NaiveTime) -> f64 {
    let elapsed = now.hour() * 3600 + now.minute() * 60 + now.second();
    elapsed as f64 / 86400.0
}

pub struct DayTotals {
    pub protein: f64,
    pub fiber: f64,
}

pub struct DayTargets {
    pub max_calories: Option<u32>,
    pub min_protein: Option<f64>,
    pub min_fiber: Option<f64>,
    pub maintenance_calories: Option<u32>,
}

pub fn render_day_summary(
    now: Option<NaiveTime>,
    totals: &DayTotals,
    exercise_calories: u32,
    net_calories: f64,
    targets: &DayTargets,
    deficit: Option<f64>,
) -> String {
    let is_today = now.is_some();
    let dp = now.as_ref().map(day_proportion).unwrap_or(1.0);

    let mut line1: Vec<String> = Vec::new();

    if let Some(target) = targets.max_calories {
        let color = if net_calories > target as f64 {
            ANSI_BOLD_RED
        } else if !is_today {
            ANSI_BOLD_GREEN
        } else {
            ANSI_BOLD_YELLOW
        };
        line1.push(format!(
            "Calories: {color}{:.0}{}/{}",
            net_calories, ANSI_RESET, target
        ));
    }
    if let Some(target) = targets.min_protein {
        let color = if totals.protein >= target {
            ANSI_BOLD_GREEN
        } else if !is_today {
            ANSI_BOLD_RED
        } else {
            let ratio = totals.protein / target;
            if ratio >= dp {
                ANSI_BOLD_YELLOW
            } else {
                ANSI_BOLD_RED
            }
        };
        line1.push(format!(
            "Protein: {color}{:.1}{}/{}g",
            totals.protein, ANSI_RESET, target
        ));
    }
    if let Some(target) = targets.min_fiber {
        let color = if totals.fiber >= target {
            ANSI_BOLD_GREEN
        } else if !is_today {
            ANSI_BOLD_RED
        } else {
            let ratio = totals.fiber / target;
            if ratio >= dp {
                ANSI_BOLD_YELLOW
            } else {
                ANSI_BOLD_RED
            }
        };
        line1.push(format!(
            "Fiber: {color}{:.1}{}/{}g",
            totals.fiber, ANSI_RESET, target
        ));
    }

    let mut line2: Vec<String> = Vec::new();

    if exercise_calories > 0 {
        line2.push(format!(
            "Exercise: {ANSI_BOLD_RED}{}{ANSI_RESET}",
            exercise_calories
        ));
    }
    if let Some(mc) = targets.maintenance_calories {
        let tdee = mc + exercise_calories;
        line2.push(format!("TDEE: {}", tdee));
    }
    if let Some(d) = deficit {
        let color = if d >= 0.0 {
            ANSI_BOLD_GREEN
        } else {
            ANSI_BOLD_RED
        };
        line2.push(format!("Deficit: {color}{:.0}{}", d, ANSI_RESET));
    }

    let max_len = line1.len().max(line2.len());
    for i in 0..max_len {
        let w1 = line1.get(i).map(|s| visible_width(s)).unwrap_or(0);
        let w2 = line2.get(i).map(|s| visible_width(s)).unwrap_or(0);
        let mw = w1.max(w2);
        if let Some(s) = line1.get_mut(i) {
            let pad = mw - visible_width(s);
            for _ in 0..pad {
                s.push(' ');
            }
        }
        if let Some(s) = line2.get_mut(i) {
            let pad = mw - visible_width(s);
            for _ in 0..pad {
                s.push(' ');
            }
        }
    }

    let mut out = String::new();
    if !line1.is_empty() {
        out.push_str(&format!("  {}", line1.join("    ")));
        out.push('\n');
    }
    if !line2.is_empty() {
        out.push_str(&format!("  {}", line2.join("    ")));
        out.push('\n');
    }
    out
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
        assert!(md.contains("\u{1b}[1;32mTotal                    320       18.0g"));
        assert!(md.contains("\u{1b}[1;36mPer serving              160        9.0g"));
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

    #[test]
    fn test_visible_width_with_ansi() {
        assert_eq!(visible_width("\x1b[1;32mhello\x1b[0m"), 5);
    }

    #[test]
    fn test_visible_width_empty() {
        assert_eq!(visible_width(""), 0);
    }
}
