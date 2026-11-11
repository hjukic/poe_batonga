# Windows minimap adapter

The Windows implementation captures the visible minimap using the OS GDI desktop
surface. It reads no game memory, modifies no client files and sends no game
inputs. This is an experimental windowed/borderless adapter, not confirmation
that navigation is permitted by GGG or accurate for every zone.

## Startup and use

Map reading starts automatically on Windows and Linux. The owner requested this
as core application behavior, with no on/off setting. Old `readMinimap` and
`windowsCaptureOptIn` keys are ignored and omitted when settings are next saved;
other preferences remain intact. Quit the application to stop its reader.

Use the normal top-right minimap in a windowed/borderless PoE 2 client. This
adapter does not open the map or move the player. It verifies both the foreground
title and the executable name using limited process-metadata access. It captures
only the minimap corner while that client is foreground and visible.

Coordinates come from the physical client area, not the entire monitor, using a
thread-local DPI-awareness context. Negative monitor coordinates are supported.
Focus, geometry, virtual desktop bounds and overlapping top-level windows are
checked before each frame; a frame is discarded if focus/geometry changes during
the copy. Moving or resizing causes the region to be located again. If another
window, including our overlay, covers the corner, capture pauses.

Raw frames remain in memory. The existing layout recorder may save reconstructed
explored geometry locally. Losing the current player
observation hides guidance and clears the drawn minimap so stale arrows are not
left visible.

## Limits and verification

- Exclusive fullscreen, HDR/color changes, custom HUD/minimap placement and
  overlays rendered inside the game have not been validated. GDI may return black
  pixels in unsupported display modes; try borderless rather than adding hooks.
- Foreground checks are best-effort around a desktop blit, not a guarantee that
  every possible transient overlay can be detected.
- Capture support does not establish accurate player detection, layout matching
  or exit prediction. The existing recognizer still needs labelled gameplay tests.
- No live gameplay capture was performed during development verification. On
  startup the reader waits for a foreground, unobstructed PoE 2 window.

Windows automated tests: physical crop calculations (including negative origins
and rounding), overlap boundaries, legacy-settings compatibility, and repeated
offscreen GDI bitmap copies checking RGB channels, crop offsets and row order.
The offscreen test does not capture the user's desktop or the game.

Manual acceptance still needs windowed and borderless PoE 2 at different scales,
monitor positions, Alt+Tab, minimization, resizing, a covered minimap and automatic
resume. Verify real marker/line detection separately from capture success.

OS references: [Microsoft GDI capture](https://learn.microsoft.com/en-us/windows/win32/gdi/capturing-an-image)
and [client-area coordinates](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getclientrect).
Windows.Graphics.Capture remains a possible future adapter if a demonstrated
limitation of this visible-desktop approach requires it.
