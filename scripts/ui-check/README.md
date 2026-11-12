# UI checks: screenshots, contrast and the real window

Dev tools for the quality bar in CLAUDE.md ("verify in both themes, at 900x600 and 1400 wide; contrast WCAG AA; keyboard;
the real window for anything that touches IPC"). They are not part of the app or of CI. Written for the Services page
(M1.4 task 7a); for another page add its scenarios to a file like `services-scenarios.mjs` and its texts to `contrast.mjs`.

| Script | What it proves |
|---|---|
| `services-shots.mjs` | Screenshots of every state (data, leftovers notice, empty, error, loading), both themes, 1400 and 900 wide, in a browser with a fake Tauri backend. **Look at the images.** |
| `contrast.mjs` | WCAG contrast (4.5:1) of every kind of text on the page, both themes, from the colours the browser really computed. Exits 1 on a failure. |
| `seed-home.mjs` + `real-window.ps1` | The **real** app window with a seeded throw-away `LUNA_HOME`: the command is permitted, the data is right, both themes at the real minimum size, and closing the window ends the process. Windows only. |

## Browser checks (screenshots and contrast)

1. A scratch folder **outside the repo** with Playwright: `npm init -y && npm i playwright-core`. It is not a dependency of lunaRun.
2. Edge must be installed (or set `EDGE_PATH`). Set `PLAYWRIGHT_CORE` to that folder's
   `node_modules/playwright-core/index.mjs`.
3. Start the dev server **on the IPv4 address**: `pnpm exec vite --host 127.0.0.1 --port 1420 --strictPort` (plain `vite` listened on
   IPv6 only here and then hung with connections stuck in `CLOSE_WAIT`). Set `UI_URL` for another address.
4. `node scripts/ui-check/services-shots.mjs` (images go to `scripts/ui-check/out/` or `OUT_DIR`) and
   `node scripts/ui-check/contrast.mjs`.
5. Stop the dev server afterwards (`Get-NetTCPConnection -LocalPort 1420`).

Things that cost time once:

- **The first page load against a cold dev server is very slow** on a slow PC (25 seconds in one run; in another the very first run still timed out after four minutes): the scripts wait that long, and if the first run after starting Vite does time out, **run it again**: the second run is fast.
- Use `waitUntil: "domcontentloaded"`, not `load`.
- A page that needs a backend answer must be given the fake bridge (`bridge()` in the scenarios file): without it every call
  fails with "The lunaRun backend is not available".

## The real window (Windows)

1. `pnpm tauri build --no-bundle` (about 10 minutes on a slow PC, incremental).
2. `node scripts/ui-check/seed-home.mjs <parent folder>` makes `<parent>/lüna home`.
3. `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/ui-check/real-window.ps1 -Exe <repo>\target\release\lunarun.exe -HomeParent <parent> -OutDir <folder>`
   (the script finds the seeded folder under the parent itself: a `ü` in a command-line argument gets mangled between shells).

It prints what the window shows (read through UI Automation: the nav link is a `Hyperlink`, the theme switch is a
`RadioButton`), saves two screenshots and reports how long the process took to exit after the window was closed.

**Window sizes are physical pixels.** The app's minimum is 900x600 logical pixels. With 125% display scaling that is 1125x750,
the script's default (`-MinWidth`, `-MinHeight`). A 900x600 physical window is smaller than the app allows and shows
clipping a user can never see.

## Contrast: what was found and how it was fixed

The first measurement found the light theme's status badges at 3.9:1 to 4.4:1 (the mid-tone `--success`, `--info` and
`--destructive` are right for fills and icons but too pale as 12px text on a tint). They now use
`--*-soft-foreground` tokens (darker in the light theme, unchanged in the dark theme). Other places that use the semantic
colours as text (`error-state`, `toast`, `registry-status`, `runtime-problem`, the uninstall dialog) have not been measured.
