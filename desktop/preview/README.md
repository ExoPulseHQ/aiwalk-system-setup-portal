# UI preview in a browser

Opens the real `desktop/ui/` pages in an ordinary browser, with invented data standing in for the Rust backend.
Meant for screenshots and design review. It changes nothing under `desktop/ui/` or `desktop/src/`.

```
node desktop/preview/serve.mjs [port]     # default 4173, Node stdlib only
```

Then open `http://localhost:4173/?as=owner&page=people`.

| Parameter | Values | Effect |
|---|---|---|
| `as` | `owner` (default), `member`, `signedout`, `terms` | who is signed in; `terms` = signed in, terms not yet accepted |
| `page` | `team`, `machines`, `people`, `terms`, `android`, `vm` | opens that page after load |
| `os` | `linux` (default), `windows`, `macos` | what `platform` and `tools` report; non-Linux hides the phone and VM pages |
| `design` | `next` | lays the proposed restyle over the real pages: `next.css` after the page's CSS, `next.js` after its scripts (fonts in `fonts/`, IBM Plex, OFL; icons in `icons.js`, Lucide, ISC). Without it nothing changes |
| `display` | `a` (default), `b`, `c` | with `design=next`: the display face for the three words that carry identity and state (the person's name, the page's sentence, the wordmark). `a` Instrument Serif, `b` Young Serif, `c` Bricolage Grotesque; all SIL OFL 1.1, Latin subsets from the @fontsource packages v5.3.0, notices in `fonts/LICENSE.txt`. Chinese falls through to Noto Serif TC / Source Han Serif TC / Songti TC / PMingLiU (a, b) or IBM Plex Sans TC / Noto Sans TC / PingFang TC / Microsoft JhengHei (c) |
| `theme` | `light` | forces the light theme. With `design=next`, `theme=dark` forces dark too. Dark follows `prefers-color-scheme` only, so for dark use the browser: Chrome headless `--force-dark-mode`, or DevTools > Rendering > Emulate prefers-color-scheme |

With `design=next`, icons carry states and row actions and the words move into one tooltip (hover after 120 ms, keyboard focus at once, Escape closes);
the Windows VM page shows the app's own shortcut icons, which `serve.mjs` serves from the repo's `icons/` at `/__icons/`.

Files: `serve.mjs` (server; injects `mock.js` before the first `<script` of `index.html`), `mock.js` (fake `window.__TAURI__`, 150 ms answers),
`fixtures.js` (one invented answer per command). A command without a fixture logs `preview: no fixture for <cmd>` and resolves `null`.
Action commands (invite, set_access, sign_out, ...) answer with a success text and change nothing; the page redraws from the same fixtures.
Tauri events (`listen`) never fire, so progress meters stay at their first text.

All data is made up (`ExampleLab`, `amy-chen`, `@example.com`); keep it that way, the fixtures are committed.
