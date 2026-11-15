/// Thumbnail dimensions exclude the title, button padding and grid spacing.
#[derive(Debug, Clone, Copy)]
pub struct Grid {
    pub columns: u8,
    pub width: i32,
    pub height: i32,
}

const MAX_WIDTH: f64 = 360.0;
const MAX_HEIGHT: f64 = 225.0;
const CARD_X: i32 = 24;
const CARD_Y: i32 = 48;

/// Fit the largest equal-size previews into the monitor's logical dimensions.
/// Prefer the configured column count when several grids fit full-size cards.
pub fn fit(count: usize, screen_width: i32, screen_height: i32, preferred: u8) -> Grid {
    let count = count.max(1);
    let available_width = (screen_width - 64).max(1);
    let available_height = (screen_height - 96).max(1);
    let preferred = preferred.max(1);
    let mut best = Grid {
        columns: 1,
        width: 1,
        height: 1,
    };
    let mut best_scale = -1.0;
    for columns in 1..=count.min(255) {
        let rows = count.div_ceil(columns).min(i32::MAX as usize) as i32;
        let width = available_width / columns as i32 - CARD_X;
        let height = available_height / rows - CARD_Y;
        let scale = (f64::from(width) / MAX_WIDTH)
            .min(f64::from(height) / MAX_HEIGHT)
            .clamp(0.0, 1.0);
        let closer = (columns as u8).abs_diff(preferred) < best.columns.abs_diff(preferred);
        if scale > best_scale || (scale == best_scale && closer) {
            best_scale = scale;
            best = Grid {
                columns: columns as u8,
                width: (MAX_WIDTH * scale).floor().max(1.0) as i32,
                height: (MAX_HEIGHT * scale).floor().max(1.0) as i32,
            };
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_large_cards_until_they_need_to_shrink() {
        let grid = fit(8, 1600, 1000, 4);
        assert_eq!((grid.columns, grid.width, grid.height), (4, 360, 225));
        let crowded = fit(24, 1600, 1000, 4);
        assert!(crowded.width < grid.width && crowded.height < grid.height);
    }

    #[test]
    fn fits_grid_and_titles_on_small_scaled_and_portrait_monitors() {
        for (width, height) in [(800, 600), (1600, 1000), (1920, 1080), (900, 1440)] {
            for count in 1usize..=100 {
                let grid = fit(count, width, height, 4);
                let rows = count.div_ceil(usize::from(grid.columns)) as i32;
                assert!(
                    i32::from(grid.columns) * (grid.width + CARD_X) <= width - 64,
                    "{count} cards on {width}x{height}: {grid:?}"
                );
                assert!(
                    rows * (grid.height + CARD_Y) <= height - 96,
                    "{count} cards on {width}x{height}: {grid:?}"
                );
            }
        }
    }

    #[test]
    fn adding_windows_never_enlarges_the_cards() {
        for (width, height) in [(800, 600), (1600, 1000), (1920, 1080)] {
            let mut previous = i32::MAX;
            for count in 1..=100 {
                let grid = fit(count, width, height, 4);
                let area = grid.width * grid.height;
                assert!(area <= previous);
                previous = area;
            }
        }
    }

    #[test]
    fn handles_empty_lists_and_invalid_preferences() {
        let grid = fit(0, 1600, 1000, 0);
        assert_eq!((grid.columns, grid.width, grid.height), (1, 360, 225));
        let tiny = fit(2, 1, 1, 4);
        assert!(tiny.width > 0 && tiny.height > 0 && tiny.columns > 0);
    }
}
