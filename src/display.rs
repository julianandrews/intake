use crate::amount::{round_away, Calories};
use crate::config::{Column, ColumnTarget};
use anyhow::{anyhow, Result};
use chrono::{NaiveTime, Timelike};
use std::fmt::Write;

pub use rust_decimal::Decimal;
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

fn day_proportion(now: &NaiveTime) -> Decimal {
    let elapsed = now.hour() * 3600 + now.minute() * 60 + now.second();
    Decimal::from(elapsed) / Decimal::from(86400)
}

pub fn wrap_color(value: &str, color: Option<&str>) -> String {
    match color {
        Some(color) => format!("{color}{value}{ANSI_RESET}"),
        None => value.to_string(),
    }
}

pub trait ColumnValue {
    fn column_value(&self, column: Column) -> Decimal;
}

macro_rules! impl_column_value {
    ($t:ty, $calories:ident, $protein:ident, $fiber:ident, $fat:ident, $carbs:ident, $alcohol:ident) => {
        impl $crate::display::ColumnValue for $t {
            fn column_value(&self, column: $crate::config::Column) -> $crate::display::Decimal {
                match column {
                    $crate::config::Column::Calories => self.$calories.into(),
                    $crate::config::Column::Protein => self.$protein.into(),
                    $crate::config::Column::Fiber => self.$fiber.into(),
                    $crate::config::Column::Fat => self.$fat.into(),
                    $crate::config::Column::Carbs => self.$carbs.into(),
                    $crate::config::Column::Alcohol => self.$alcohol.into(),
                }
            }
        }
    };
}
pub(crate) use impl_column_value;

#[derive(Debug, Clone, Copy, Default)]
pub struct DayTotals {
    pub calories: Decimal,
    pub protein: Decimal,
    pub fiber: Decimal,
    pub fat: Decimal,
    pub carbs: Decimal,
    pub alcohol: Decimal,
}

impl_column_value!(DayTotals, calories, protein, fiber, fat, carbs, alcohol);

impl DayTotals {
    fn slot_mut(&mut self, column: Column) -> &mut Decimal {
        match column {
            Column::Calories => &mut self.calories,
            Column::Protein => &mut self.protein,
            Column::Fiber => &mut self.fiber,
            Column::Fat => &mut self.fat,
            Column::Carbs => &mut self.carbs,
            Column::Alcohol => &mut self.alcohol,
        }
    }

    /// Accumulate one row's macro values; `None` on overflow.
    pub fn checked_add_row(&mut self, row: &impl ColumnValue) -> Option<()> {
        for column in Column::all() {
            let slot = self.slot_mut(column);
            *slot = slot.checked_add(row.column_value(column))?;
        }
        Some(())
    }
}

pub fn column_color(
    now: Option<NaiveTime>,
    value: Decimal,
    target: &ColumnTarget,
) -> Option<&'static str> {
    let dp = now.as_ref().map(day_proportion).unwrap_or(Decimal::ONE);

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

fn rescale(mut value: Decimal, places: u32) -> Decimal {
    value.rescale(places);
    value
}

pub fn log_cell(column: Column, value: Decimal) -> String {
    match column {
        Column::Calories => rescale(round_away(value, 0), 0).to_string(),
        _ => rescale(round_away(value, 1), 1).to_string(),
    }
}

pub fn food_cell(column: Column, value: Decimal) -> String {
    match column {
        Column::Calories => rescale(round_away(value, 0), 0).to_string(),
        _ => format!("{}g", rescale(round_away(value, 1), 1)),
    }
}

pub fn servings_cell(servings: Decimal) -> String {
    if servings.fract().is_zero() {
        servings.round_dp(0).to_string()
    } else {
        round_away(servings, 1).to_string()
    }
}

pub fn render_day_summary(
    exercise_calories: Calories,
    maintenance_calories: Option<Calories>,
    deficit: Option<Decimal>,
) -> Result<String> {
    let mut lines: Vec<(String, String)> = Vec::new();

    if let Some(mc) = maintenance_calories {
        let tdee = mc
            .checked_add(exercise_calories)
            .ok_or_else(|| anyhow!("TDEE overflow"))?;
        lines.push((
            "TDEE:".to_string(),
            format!("{}", round_away(tdee.to_decimal(), 0)),
        ));
    }
    if let Some(d) = deficit {
        lines.push(("Deficit:".to_string(), round_away(d, 0).to_string()));
    }

    if lines.is_empty() {
        Ok(String::new())
    } else {
        let label_width = lines
            .iter()
            .map(|(label, _)| visible_width(label))
            .max()
            .unwrap();
        let value_width = lines
            .iter()
            .map(|(_, value)| visible_width(value))
            .max()
            .unwrap();
        let mut out = String::from("\n");
        for (label, value) in lines {
            let pad = " ".repeat(label_width - visible_width(&label));
            writeln!(
                out,
                "{ANSI_BOLD_MAGENTA}{label}{ANSI_RESET}{pad}  {value:>value_width$}"
            )
            .unwrap();
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DayTargets;
    use std::str::FromStr;

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
                max: Some(Decimal::from(2000)),
            },
            protein: ColumnTarget {
                min: Some(Decimal::from(100)),
                max: None,
            },
            fiber: ColumnTarget {
                min: Some(Decimal::from(20)),
                max: None,
            },
            ..DayTargets::default()
        }
    }

    #[test]
    fn test_column_color_past_day() {
        assert_eq!(
            column_color(None, Decimal::from(2100), &targets().calories),
            Some(ANSI_RED)
        );
        assert_eq!(
            column_color(None, Decimal::from(90), &targets().protein),
            Some(ANSI_YELLOW)
        );
        assert_eq!(
            column_color(None, Decimal::from(25), &targets().fiber),
            Some(ANSI_GREEN)
        );
    }

    #[test]
    fn test_column_color_today_at_noon() {
        let now = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        let calories = targets().calories;

        // net 800: at/below half of 2000 -> green
        assert_eq!(
            column_color(Some(now), Decimal::from(800), &calories),
            Some(ANSI_GREEN)
        );

        // net 1200: over half, under target -> yellow
        assert_eq!(
            column_color(Some(now), Decimal::from(1200), &calories),
            Some(ANSI_YELLOW)
        );

        // net 2500: over target -> red
        assert_eq!(
            column_color(Some(now), Decimal::from(2500), &calories),
            Some(ANSI_RED)
        );

        // protein 40 >= 50? no -> yellow; fiber 5 >= 10? no -> yellow
        assert_eq!(
            column_color(Some(now), Decimal::from(40), &targets().protein),
            Some(ANSI_YELLOW)
        );
        assert_eq!(
            column_color(Some(now), Decimal::from(5), &targets().fiber),
            Some(ANSI_YELLOW)
        );
    }

    #[test]
    fn test_column_color_min_and_max_band() {
        let target = ColumnTarget {
            min: Some(Decimal::from(50)),
            max: Some(Decimal::from(90)),
        };
        // below min -> yellow
        assert_eq!(
            column_color(None, Decimal::from(40), &target),
            Some(ANSI_YELLOW)
        );
        // at min -> green
        assert_eq!(
            column_color(None, Decimal::from(50), &target),
            Some(ANSI_GREEN)
        );
        // in band -> green
        assert_eq!(
            column_color(None, Decimal::from(70), &target),
            Some(ANSI_GREEN)
        );
        // above max -> red
        assert_eq!(
            column_color(None, Decimal::from(95), &target),
            Some(ANSI_RED)
        );
    }

    #[test]
    fn test_column_color_band_scales_with_day_progress() {
        let target = ColumnTarget {
            min: Some(Decimal::from(50)),
            max: Some(Decimal::from(90)),
        };
        let now = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
        // band halves at noon: 25..45
        assert_eq!(
            column_color(Some(now), Decimal::from(20), &target),
            Some(ANSI_YELLOW)
        );
        assert_eq!(
            column_color(Some(now), Decimal::from(30), &target),
            Some(ANSI_GREEN)
        );
        // between scaled max (45) and full max (90) -> yellow
        assert_eq!(
            column_color(Some(now), Decimal::from(50), &target),
            Some(ANSI_YELLOW)
        );
        // above full max -> red
        assert_eq!(
            column_color(Some(now), Decimal::from(95), &target),
            Some(ANSI_RED)
        );
    }

    #[test]
    fn test_column_color_no_targets() {
        let target = ColumnTarget::default();
        assert_eq!(column_color(None, Decimal::from(2100), &target), None);
    }

    #[test]
    fn test_render_day_summary() {
        let out = render_day_summary(
            Calories::from_u32(300),
            Some(Calories::from_u32(2400)),
            Some(Decimal::from(1500)),
        )
        .unwrap();
        assert!(out.contains(&format!("{ANSI_BOLD_MAGENTA}TDEE:{ANSI_RESET}")));
        assert!(out.contains(&format!("{ANSI_BOLD_MAGENTA}Deficit:{ANSI_RESET}")));
        assert!(out.contains("2700"));
        assert!(out.contains("1500"));
        assert!(!out.contains(ANSI_GREEN));
        assert!(!out.contains(ANSI_RED));
    }

    #[test]
    fn test_render_day_summary_fractional_exercise_rounds() {
        let out = render_day_summary(
            Calories::from_str("300.5").unwrap(),
            Some(Calories::from_u32(2400)),
            Some(Decimal::from_str("1201.5").unwrap()),
        )
        .unwrap();
        assert!(out.contains("2701"));
        assert!(out.contains("1202"));
        assert!(!out.contains("2700.5"));
    }

    #[test]
    fn test_render_day_summary_empty() {
        assert_eq!(render_day_summary(Calories::ZERO, None, None).unwrap(), "");
    }

    #[test]
    fn test_render_day_summary_negative_deficit() {
        let out = render_day_summary(
            Calories::ZERO,
            Some(Calories::from_u32(2400)),
            Some(Decimal::from(-500)),
        )
        .unwrap();
        assert!(out.contains(&format!("{ANSI_BOLD_MAGENTA}Deficit:{ANSI_RESET}  -500")));
    }

    #[test]
    fn test_column_color_past_day_below_targets() {
        assert_eq!(
            column_color(None, Decimal::from(1500), &targets().calories),
            Some(ANSI_GREEN)
        );
        assert_eq!(
            column_color(None, Decimal::from(50), &targets().protein),
            Some(ANSI_YELLOW)
        );
        assert_eq!(
            column_color(None, Decimal::from(10), &targets().fiber),
            Some(ANSI_YELLOW)
        );
    }

    #[test]
    fn test_log_cell_formats() {
        assert_eq!(
            log_cell(Column::Calories, Decimal::from_str("1500.4").unwrap()),
            "1500"
        );
        assert_eq!(
            log_cell(Column::Protein, Decimal::from_str("12.34").unwrap()),
            "12.3"
        );
        assert_eq!(
            log_cell(Column::Fat, Decimal::from_str("7.0").unwrap()),
            "7.0"
        );
        assert_eq!(
            log_cell(Column::Alcohol, Decimal::from_str("2.5").unwrap()),
            "2.5"
        );
    }

    #[test]
    fn test_food_cell_formats() {
        assert_eq!(food_cell(Column::Calories, Decimal::from(160)), "160");
        assert_eq!(food_cell(Column::Protein, Decimal::from(9)), "9.0g");
        assert_eq!(
            food_cell(Column::Carbs, Decimal::from_str("30.25").unwrap()),
            "30.3g"
        );
    }

    #[test]
    fn test_servings_cell_formats() {
        assert_eq!(servings_cell(Decimal::from(2)), "2");
        assert_eq!(servings_cell(Decimal::from_str("2.0").unwrap()), "2");
        assert_eq!(servings_cell(Decimal::from_str("1.5").unwrap()), "1.5");
        assert_eq!(servings_cell(Decimal::from_str("1.25").unwrap()), "1.3");
    }

    #[test]
    fn test_rounding_strategy_consistent_between_tables() {
        let value = Decimal::from_str("1500.5").unwrap();
        assert_eq!(log_cell(Column::Calories, value), "1501");
        assert_eq!(food_cell(Column::Calories, value), "1501");
        let protein = Decimal::from_str("12.35").unwrap();
        assert_eq!(log_cell(Column::Protein, protein), "12.4");
        assert_eq!(food_cell(Column::Protein, protein), "12.4g");
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
