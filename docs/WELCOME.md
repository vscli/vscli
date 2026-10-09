# Native welcome and brand artwork

An empty workbench shows the original VSCLI graphical mark, a normal wordmark, contextual shortcut actions and recent files. Actions and recent-file rows can be clicked. Keyboard shortcuts retain their existing command routing and user overrides; the welcome page does not create an editor or modify hidden documents. Recent-file clicks use the existing bounded asynchronous loader and preserve dirty/shared identities.

Settings shows the actual user settings path used by the editor, including an explicitly selected file or imported profile. Ctrl+, (Cmd+, on macOS) opens that same JSON file; the displayed shortcut follows native overrides. `.vscode/settings.json` in the current workspace provides workspace overrides. There is no mandatory configuration wizard. Extensions and Keyboard Inspector are available directly on the welcome page and through the command palette. Getting Started describes automatic installed clangd/rust-analyzer startup, custom `--lsp`, and `--no-lsp`.

## Rendering and limits

The editable original is [the SVG mark](../assets/brand/vscli-mark.svg); its derived transparent PNG is embedded. One retained worker decodes the PNG once and prepares the latest requested size/theme. Requests and results each have capacity one; resize changes coalesce while work is outstanding. Late results never install into another geometry. A two-second deadline rejects delayed preparation while retaining the occupied slot until completion. Input and rendering only record requests, poll results and paint cached data. There are no stdin capability probes, child processes, tmux configuration changes or image-file reads in rendering.

The high-resolution path requires `TERM=xterm-kitty`, a numeric `KITTY_WINDOW_ID`, no tmux/screen environment, no nonempty `NO_COLOR`, and usable terminal cell-pixel geometry. The worker reads that geometry and fits the mark to the physical canvas before encoding. The PNG is capped at 64 KiB and 512 × 512 pixels with a 4 MiB decoder allocation limit; the displayed area is at most 28 × 14 cells and the encoded physical canvas is at most 1024 × 1024 pixels. Missing or over-limit geometry uses colored Unicode half-blocks. Other terminals use that same crisp cell fallback, and cell colors obey `NO_COLOR`; small windows retain the plain wordmark and available actions.

Kitty rendering uses Unicode image placeholders, compressed native pixel data and one stable process-scoped image ID. Clearing welcome cells hides placements when opening a file or modal. A resized Kitty image deletes the prior payload immediately before replacement; normal exit and terminal restoration explicitly delete the payload. Merely hiding/showing the unchanged welcome reuses its cached image. No unbounded image-ID allocation occurs. As with other terminal cleanup, SIGKILL cannot run restoration.

## Evidence

The graphical mark and final welcome were inspected in a real Kitty 0.49.2 window on Linux, including physical cell sizing and transparency. [Kitty screenshot](images/welcome-kitty.png) and [cell fallback screenshot](images/welcome-cells.png) show the two paths. These checks do not establish arbitrary terminal or multiplexer graphics parity.

Native tests cover encoded/decode/cell/pixel budgets, conservative mode detection, occupied-worker coalescing and late replies, process-ID reuse and scoped replacement, image hide/show, responsive layouts, shortcut overrides, actual settings paths, click dispatch and dirty hidden-buffer preservation. `tests/welcome_brand_pty.py` runs real fallback and Kitty-wire PTYs through welcome/help/open/edit/cancel/save/undo/close/recent/resize/settings/exit. The Kitty PTY supplies cell pixel dimensions and observes transmission/deletion; it is a protocol lifecycle oracle, while the screenshot is the actual graphical-terminal observation.

The graphics contract follows the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) and uses [ratatui-image 11.1.0](https://docs.rs/ratatui-image/11.1.0/ratatui_image/) with default features disabled. Only direct cached Kitty/half-block constructors are used.
