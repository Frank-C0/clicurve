# Changelog - Auto-Rotate Dashboard and Performance Optimization

This is a fork of `clicurve`, and these changes improve usability for hands-free dashboard widgets and optimize background processing.

## [Unreleased] - 2026-05-29

### Added

- **Auto-Rotate Dashboard Mode (`R`)**:
  - Cycles through selected metrics automatically (default interval: 5 seconds).
  - Built from currently selected metrics when activated, with a fallback to all visible metrics if none are selected.
  - Automatically hides the left sidebar panels (experiments/metrics list) to maximize screen real estate when active, restoring them when disabled.
  - Dynamically resizes the line chart to fill the extra space.
- **Dynamic Last-Value Display on Chart Title**:
  - Automatically extracts and displays the last/current value of the plotted metric inside the chart title bar (e.g. `Chart: train/loss (last = 0.0432)`).
  - Handles multi-experiment views by rendering the individual last values of all selected experiments together (e.g. `Chart: train/loss (expA: 0.0432 | expB: 0.0812)`).
- **Manual Rotation Control**:
  - **Stepping (`[` and `]`)**: Cycle backward and forward manually through the rotation list. Manual steps automatically reset the rotation timer.
  - **Interval Adjustment (`+` / `-`)**: Increase and decrease the rotation interval in real-time between 1 and 120 seconds.
- **Configurable Auto-Reload Period (`--reload-interval`)**:
  - Added a CLI parameter `--reload-interval` to specify the TensorBoard data auto-reload rate.
  - Bounded with a default of 30 seconds and a safe minimum clamp of 10 seconds.
- **Event File Metadata Caching**:
  - Introduced an in-place `reload()` routine in `MultiStore` and `MetricStore` that tracks filesystem metadata (file size and modified timestamp) of event files.
  - Preserves previously parsed scalar data in memory (`file_cache`). Files that are unchanged completely skip disk read, decompression, and Protobuf decoding, decreasing reload latency from seconds to **<1ms** and eliminating CPU/memory spikes.

### Changed

- ** TUI Event Loop & Redraw Optimization**:
  - Refactored `run_loop()` in `src/main.rs` to only perform TUI draws via `terminal.draw()` when `app.needs_redraw` is flagged, significantly reducing idle CPU usage.
  - Integrated dynamic polling timeouts in `src/app.rs` (`compute_poll_timeout()`) based on remaining seconds until the next rotation tick or background reload, keeping the app idle until events actually occur.
- **TUI Keyboard Overrides**:
  - Ignored irrelevant keys (e.g., arrow keys, panels tab, Space toggles, filter keys) when auto-rotation is active to prevent user interactions from breaking the hidden sidebar lists.
  - Preserved standard key navigation and filter controls when rotation is deactivated.
