use serde::Serialize;
use tauri::{Manager, WebviewWindow};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardLayout {
    width: f64,
    max_height: f64,
}

fn layout_for_work_area(width: f64, height: f64, scale: f64) -> CardLayout {
    CardLayout {
        width: (width / scale - 40.0).clamp(1.0, 500.0),
        max_height: (height / scale - 40.0).max(1.0),
    }
}

#[tauri::command]
pub fn get_card_layout(window: WebviewWindow) -> Result<CardLayout, String> {
    let monitor = window.current_monitor().map_err(|e| e.to_string())?
        .ok_or_else(|| "Card monitor is unavailable".to_string())?;
    let area = monitor.work_area();
    Ok(layout_for_work_area(area.size.width as f64, area.size.height as f64, monitor.scale_factor()))
}

#[tauri::command]
pub fn resize_card(window: WebviewWindow, height: f64) -> Result<(), String> {
    if window.label() != "main" || !height.is_finite() || height <= 0.0 {
        return Err("Invalid card resize request".to_string());
    }
    let layout = get_card_layout(window.clone())?;
    window.set_size(tauri::LogicalSize::new(layout.width, height.min(layout.max_height)))
        .map_err(|e| e.to_string())?;
    let position = window.state::<crate::state::AppState>().settings.lock().unwrap().position.clone();
    crate::reposition_window(&window, &position);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_card_keeps_width_but_can_grow_vertically() {
        let layout = layout_for_work_area(1920.0, 1040.0, 1.0);
        assert_eq!(layout.width, 500.0);
        assert_eq!(layout.max_height, 1000.0);
    }

    #[test]
    fn card_fits_small_hidpi_work_area() {
        let layout = layout_for_work_area(800.0, 600.0, 2.0);
        assert_eq!(layout.width, 360.0);
        assert_eq!(layout.max_height, 260.0);
    }
}
