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
| `theme` | `light`, `dark` | forces that theme (sets `data-theme`, which `app.css` honours); without it the page follows `prefers-color-scheme` |

The page is the app's own UI as it ships (`app.css`, `icons.js`, `fonts/`, `icons/`), so what the preview shows is what the app draws.

Files: `serve.mjs` (server; injects `mock.js` before the first `<script` of `index.html`), `mock.js` (fake `window.__TAURI__`, 150 ms answers),
`fixtures.js` (one invented answer per command). A command without a fixture logs `preview: no fixture for <cmd>` and resolves `null`.
Action commands (invite, set_access, sign_out, ...) answer with a success text and change nothing; the page redraws from the same fixtures.
Tauri events (`listen`) never fire, so progress meters stay at their first text.

All data is made up (`ExampleLab`, `amy-chen`, `@example.com`); keep it that way, the fixtures are committed.
