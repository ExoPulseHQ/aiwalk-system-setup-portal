# aIwalk System Setup: design system

The approved look of the desktop app (`desktop/ui/`), signed off by the client ("UI OK") on 2026-10-05.
The values below live in `desktop/ui/app.css`; this file says what they are and why. Read it before any visual or UI change.

## Product context

A Tauri desktop app a research team uses to sign in once (GitHub, then the team's machines through Cloudflare), keep its document vaults on the computer, see who may open what, and reach the team's machines, a Windows VM and an Android phone.
People open it to check a state and occasionally to act. Window 980x720 by default; it must work offline (fonts and icons are bundled, nothing loads from a network).

## Aesthetic direction

A calm instrument panel. Type, alignment and hairlines do the work; colour is a signal, not decoration.
One left-aligned column of 720px of content. Hairline rows, not cards. Each page opens with a strip of icons and figures that reads like an instrument's read-out.
Icons carry states and actions; words appear on hover and focus through one tooltip, and stay visible only where they matter (see Icons).

## Typography

| Face | Files (`desktop/ui/fonts/`, SIL OFL 1.1) | Role |
|---|---|---|
| IBM Plex Sans 400 / 500 / 600 | `IBMPlexSans-*-Latin1.woff2` | everything in the UI, and the display words |
| IBM Plex Mono 400 / 500 | `IBMPlexMono-*-Latin1.woff2` | machine names, logins (the chip), paths, addresses, the version, meter numbers |
| Times New Roman, 標楷體 (DFKai-SB, BiauKai, Kaiti TC) | system | the terms text and its language switch only: deliberate, a formal text keeps formal faces |

Chinese falls through to IBM Plex Sans TC, then the system's sans. Fallback stack: `"IBM Plex Sans", "IBM Plex Sans TC", system-ui, sans-serif`.

Type scale (ratio about 1.2 from a 14px body), line height 1.5, 1.25 for headings:

| Token | Size | Used for |
|---|---|---|
| `--fs-xs` | 12px | chips, meter labels, tooltip, the sidebar count |
| `--fs-s` | 13px | secondary lines, controls, buttons, the page-name label |
| `--fs-m` | 14px | body, row titles, h3 |
| `--fs-l` | 16px | section titles (h2), a machine's name chip |
| `--fs-xl` | 20px | dialog titles, h1 without a strip |
| `--fs-xxl` | 28px | the h1 of an empty state (missing tools) |
| display, strip figures | 32px / 1.1, 600, tracking -0.025em, tabular lining figures | the opening strip's numbers |
| display, name | 26px / 1.1, 600, -0.025em | the signed-in person's name |
| display, wordmark | 22px / 28px, 600, -0.025em | "aIwalk" in the sidebar (Plex's capital I has serifs, so it never reads "alwalk") |

Weights: 400 body, 500 row titles, buttons and chips, 600 headings and display. Never 700 (it maps to 600).

## Colour

Roles, not hues. Light and dark switch on `prefers-color-scheme`; `data-theme="light|dark"` on `<html>` forces one (the preview uses it).

| Token | Light | Dark | Role |
|---|---|---|---|
| `--paper` | #F7F6FA | #1E1A27 | page background |
| `--surface` | #FFFFFF | #26212F | controls, chips, panels, dialogs |
| `--ink` | #1E1A29 | #ECE8F3 | text |
| `--muted` | #6B6578 | #A49DB3 | secondary text, icons beside text |
| `--line` | #E4E0EC | #3A3347 | hairlines, chip frames, off switches |
| `--rail` / `--rail-ink` / `--rail-muted` | #1B1724 / #E9E5F0 / #9C95AB | #0D0B12 / #E2DCEC / #8F88A0 | the sidebar (dark in both themes) |
| `--purple` | #8C54D9 | #A97CEB | brand: selected, primary, healthy |
| `--purple-ink` | #7A42C8 | #B48CF0 | purple as small text (AA) |
| `--amber` / `--amber-ink` | #C77A12 / #9A5B0A | #E0A040 / #E0A040 | deviation that needs a look |
| `--red` / `--red-ink` | #C8402F / #B3361F | #E8735F / #EF7F6C | broken, cannot be reached |
| `--on-purple` | #FFFFFF | #0D0B12 | text on a purple fill |

Purple has exactly three roles: (1) the selected sidebar item, (2) the one primary button of a page, (3) healthy / connected / agreed (the `s-ok` state chip, the strip's healthy figures, the check glyphs).
Amber and red appear only where something deviates (`s-warn`, `pending`, `m-temporary`; `s-bad`). There is no green anywhere, not even for "connected". Hot meters (85% and over) are amber; meters otherwise are muted.

## Spacing and radius

4px unit: `--s1` 4, `--s2` 8, `--s3` 12, `--s4` 16, `--s6` 24, `--s8` 32, `--s10` 40, `--s12` 48. Row minimum height `--row` 44px; tree rows 40px.
Radius by size: `--r-control` 4px (buttons, inputs, chips, tooltip), `--r-panel` 8px (dialogs, the invite panel, the terms text, toast), `--r-large` 12px (reserved for the largest containers).

## Layout

- Sidebar 208px; page padding 40px sides, 40px top, 48px bottom; content max 720px, left aligned.
- Under 700px the sidebar becomes a 56px icon rail: words hidden, each item's words in the tooltip (`data-tip-narrow`), the subtitle and version hidden; page padding 24 / 16 / 40 / 46.
- Each page: its name (h1) becomes a 13px muted label once the opening strip is drawn under it (`setOpening`); the strip's state mark hangs 30px into the left margin so the strip keeps the page's left edge.
- Lists are hairline rows: a row's kind icon in its own column, title (500) and a second line (13px muted), controls on the right.
- Sections: a hairline on top, 32px above (24px between sections), heading 24px below the hairline.
- Machines: each machine is its own block. Its details are open by default; every machine after the first starts 48px lower under a 2px rule of ink at 22%.
- Who may use a thing (connect to a machine, merge on a repo) is one wrapping line: a quiet label per reason, said once, then the people as chips.

## Icons

Lucide outline icons (ISC licence, `desktop/ui/icons.js`), stroke `currentColor`, width 1.75 (1.6 at 18px and up, 2 in chips).
Sizes: 18px sidebar (20px in the rail), 16px rows and headings, 22px in the strip, 14px in buttons, 12-15px in chips and glyphs.
Icons beside words are `aria-hidden`; the words or an `aria-label` give the accessible name.

- Icon only (`iconButton(..., true)`, 28px square): row and section actions whose sign is conventional: Copy, Change folder, Check again / Look again, New desktop, Update host tools, Add a GitHub account, Sign out of GitHub, Remove (a person, from a row), Change password, a desktop's Disconnect and Close, the "+" that opens an add picker, the × on a chip.
- Icon plus word: Get latest, Open in Obsidian, Open viewer, Invite someone / Close, Disconnect (Cloudflare), the selected state of a sign-in line.
- Word only: the page's primary action when it has no conventional sign (Send invitation, I agree, Sign in with GitHub), Approve / Decline, Cancel invitation, Request access, Open desktop, Download.
- Deviations and to-dos always keep their words: "Can't connect", "Not yet", "Finish sign-in on Team access", "To do: …", "Often off".
- A state that is as it should be is its glyph alone (`glyph()`): Can connect, a level in the access tree, On this computer, Invited, Intern, a sign-in way not in use. A glyph with no frame only says something; a frame means it is a button.
- An explanation sentence sits behind an info mark (ⓘ) after the heading or label it explains, never as a paragraph under it.
- Tooltip: one element (`#tip`), ink on paper, 12px, max 300px, 4px radius. Opens 120ms after hover, at once on keyboard focus; closes on Escape, on leaving, on focus out, and when its element is redrawn away. Every icon-only control, glyph, strip figure and info mark has `data-tip`; icon-only controls use the same text as `aria-label`, and anything else gets `aria-describedby="tip"` while it shows.

## Ink rendering

A narrow same-hue vertical gradient as the text fill (`background-clip: text`), a glow through `filter: drop-shadow` (two layers in dark, one soft violet cast in light).
Only these elements may carry it: the wordmark (`nav .app .word`, rail recipe in both themes), the opening strip's figures (`.fig-num`: purple when healthy, amber or red when deviating, neutral ink when quiet), the strip's state mark when healthy (glow only), and the person's name (`.badge .who .name`, neutral ink).
Related, and only these two: the selected sidebar item and the page's one primary button get a lit top edge (a white 7% to 0 gradient, an inset 1px highlight, a 1px contact shadow), as a real key would.
`prefers-contrast: more` and `forced-colors: active` turn all of it flat.

## Motion

Short and functional: 150ms background and shadow on buttons, rows and tiles; 240ms `cubic-bezier(.2,.7,.2,1)` for the invite drawer (grid rows 0fr to 1fr); 180ms rise for dialogs; 150ms knob on switches and chevron on vault folds; a 2px purple line sweeping across the top while the backend works.
`prefers-reduced-motion: reduce` removes them (the top line and spinners breathe instead).

## Never

- Never the word "lab" in UI text: it is "the team".
- No green; no purple for anything but the three roles; no amber or red for anything that is not a deviation.
- No cards, drop shadows or gradients outside the ink list and the lit edge; no second primary button on a page.
- No network fonts, CDNs or remote images: the app must work offline.
- No `title` attributes for explanations (they make a second, unstyled tooltip); use `data-tip`.
- No text that only colour distinguishes: every state has a glyph and a word (visible or in `.sr` and the tooltip).
- Do not restyle the terms text away from Times New Roman and 標楷體.

## Decisions log

- 2026-10-05: The overlay approved in the browser preview (`?design=next`, display `plex`, rendering `ink`) became the app's native UI; the overlay and its choosing options (`display=a|b|c`, `fx=none|depth|engraved`) were retired. Fonts: IBM Plex Sans and Mono only (Instrument Serif, Young Serif and Bricolage were candidates, dropped).
- 2026-10-05: Machine details open by default and the row is a real disclosure (`role="button"`, `aria-expanded`, Enter and Space fold it).
- 2026-10-05: The primary button is chosen by `pickPrimary`: the first enabled `data-primary` button, else the first non-ghost button; none on the Windows VM and Android pages; with the invite drawer open it is Send invitation.
- 2026-10-05: Dialog buttons stay neutral (the overlay never coloured them); the confirming button is last.
- 2026-10-05: Logins are chips for members too (the overlay could only chip logins it knew, so a member saw most as plain text).
- 2026-10-05: The invite field has a visible "Username" label; the first sign-in page keeps the strip and hides the lede, while "Add another GitHub account" and "Unlock the owner tools" keep their explanation and have no strip.
- 2026-10-05: Guest mode (outside teams on single machines) reuses existing patterns only: guests are interns whose one repo is the requests repo, marked with the `server` glyph beside the intern's cap; a guest's Team access is the badge plus one row to Machines; "Not let in" is a deviation (`s-bad`, words kept); the owner's outside-the-team dialog reuses the invite panel's labelled fields.
- 2026-10-05: The SSH button reuses existing patterns only: an icon-only `square-terminal` button in the machine's heading beside the state glyph (shown while the machine is up or a sign-in away, and only where the app can start a terminal: `platform` is linux, macos or windows), and one `spec hw` line in the card with the command and Copy; a failed launch puts its sentence (`warn`) above that line.
