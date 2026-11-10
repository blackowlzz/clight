const SPECIAL = {
  Enter: '\r', Backspace: '\x7f', Tab: '\t', Escape: '\x1b', 'S-Tab': '\x1b[Z',
  Home: '\x1b[H', End: '\x1b[F', PageUp: '\x1b[5~', PageDown: '\x1b[6~', Delete: '\x1b[3~', Insert: '\x1b[2~',
  F1: '\x1bOP', F2: '\x1bOQ', F3: '\x1bOR', F4: '\x1bOS', F5: '\x1b[15~', F6: '\x1b[17~', F7: '\x1b[18~', F8: '\x1b[19~', F9: '\x1b[20~', F10: '\x1b[21~', F11: '\x1b[23~', F12: '\x1b[24~',
}
const ARROW = { ArrowUp: 'A', ArrowDown: 'B', ArrowRight: 'C', ArrowLeft: 'D' }

export const TERMINAL_PASS = new Set(['A-F12', 'A-1', 'A-7', 'A-0', 'A-3', 'A-b', 'A-4', 'A-5', 'A-6', 'F11', 'C-F9', 'S-F10', 'S-F9', 'C-F2', 'C-p', 'C-S-a'])

export function termSequence(e, name, appCursor) {
  if (ARROW[e.key]) {
    const mod = 1 + (e.shiftKey ? 1 : 0) + (e.altKey ? 2 : 0) + (e.ctrlKey ? 4 : 0)
    return mod > 1 ? `\x1b[1;${mod}${ARROW[e.key]}` : `\x1b${appCursor ? 'O' : '['}${ARROW[e.key]}`
  }
  const special = SPECIAL[name] ?? (e.shiftKey ? undefined : SPECIAL[e.key])
  if (special) return special
  if (e.ctrlKey && !e.altKey && e.key.length === 1) {
    const k = e.key.toLowerCase()
    const code = k >= 'a' && k <= 'z' ? k.charCodeAt(0) - 96 : { '[': 27, '\\': 28, ']': 29, '^': 30, _: 31, ' ': 0 }[k]
    return code === undefined ? null : String.fromCharCode(code)
  }
  if (e.altKey && !e.ctrlKey && e.key.length === 1) return '\x1b' + e.key
  return null
}
