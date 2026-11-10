import { root, store, load, state } from './core.js'

const hex = /^#[0-9a-f]{6}$/i

export const TOKENS = [
  ['Interface', [
    ['bg', 'Editor background'], ['chrome', 'Panels and bars'], ['hover', 'Hover'], ['active', 'Selected item'],
    ['line', 'Dividers'], ['line-strong', 'Borders'], ['text', 'Text'], ['muted', 'Secondary text'], ['faint', 'Dim text'],
    ['accent', 'Accent'],
  ]],
  ['Editor', [
    ['code', 'Plain code'], ['sel', 'Selection'], ['mark', 'Search match'], ['num', 'Line numbers'], ['guide', 'Indent guides'],
    ['curline', 'Current line'], ['caret', 'Caret'],
  ]],
  ['Syntax', [
    ['k', 'Keywords'], ['t', 'Built-in types'], ['y', 'Classes and types'], ['f', 'Functions'], ['v', 'Fields and globals'], ['l', 'Variables and parameters'],
    ['o', 'Namespaces'], ['r', 'Strings'], ['x', 'Escape sequences'], ['n', 'Numbers'], ['z', 'true, false, null'], ['c', 'Comments'], ['d', 'Preprocessor'], ['w', 'Macros'],
  ]],
  ['Signals', [
    ['err', 'Errors'], ['warn', 'Warnings'], ['bp', 'Breakpoints'], ['git-add', 'Git added'], ['git-mod', 'Git modified'], ['run', 'Run button'], ['glow', 'Mouse glow'],
  ]],
]

const blackBase = {
  bg: '#000000', chrome: '#0c0c0c', hover: '#181818', active: '#222222', line: '#1f1f1f', 'line-strong': '#333333',
  text: '#e4e4e4', muted: '#8c8c8c', faint: '#5c5c5c', accent: '#e4e4e4',
  code: '#c4c4c4', sel: '#2e2e2e', mark: '#2a2a2a', num: '#444444', guide: '#1a1a1a', curline: '#0e0e0e', caret: '#e4e4e4',
  k: '#e8e8e8', t: '#e8e8e8', y: '#e8e8e8', f: '#e8e8e8', v: '#c4c4c4', o: '#8c8c8c', r: '#8c8c8c', n: '#8c8c8c', c: '#5c5c5c', d: '#8c8c8c', w: '#8c8c8c', x: '#cc7832',
  err: '#e4e4e4', warn: '#8c8c8c', bp: '#e4e4e4', 'git-add': '#e4e4e4', 'git-mod': '#5c5c5c', glow: '#ffffff',
}
const jbSyntax = { code: '#bcbec4', x: '#cf8e6d', k: '#cf8e6d', t: '#cf8e6d', y: '#b5b6e3', f: '#56a8f5', v: '#c77dbb', o: '#b5b6e3', r: '#6aab73', n: '#2aacb8', c: '#7a7e85', d: '#b3ae60', w: '#b3ae60', err: '#f75464', warn: '#d5b778', bp: '#db5c5c', 'git-add': '#57965c', 'git-mod': '#4a88c7' }
const paperBase = {
  bg: '#ffffff', chrome: '#f6f6f6', hover: '#ececec', active: '#e2e2e2', line: '#e3e3e3', 'line-strong': '#c8c8c8',
  text: '#1a1a1a', muted: '#6a6a6a', faint: '#9a9a9a', accent: '#1a1a1a',
  code: '#333333', sel: '#d6d6d6', mark: '#e0e0e0', num: '#bbbbbb', guide: '#eeeeee', curline: '#f6f6f6', caret: '#1a1a1a',
  k: '#111111', t: '#111111', y: '#111111', f: '#111111', v: '#333333', o: '#6a6a6a', r: '#6a6a6a', n: '#6a6a6a', c: '#9a9a9a', d: '#6a6a6a', w: '#6a6a6a', x: '#b85c00',
  err: '#1a1a1a', warn: '#6a6a6a', bp: '#1a1a1a', 'git-add': '#1a1a1a', 'git-mod': '#9a9a9a', glow: '#000000',
}

const makeTheme = (bg, chrome, hover, active, line, strong, text, muted, faint, accent, sel, num, cur, syntax, signals) => ({
  bg, chrome, hover, active, line, 'line-strong': strong, text, muted, faint, accent,
  code: syntax.code, sel, mark: active, num, guide: line, curline: cur, caret: accent, ...syntax, ...signals, glow: '#ffffff',
})

export const PRESETS = [
  { id: 'black', name: 'Black', bold: true, italic: true, colors: blackBase },
  { id: 'black-color', name: 'Black Color', colors: { ...blackBase, ...jbSyntax } },
  { id: 'paper', name: 'Paper', bold: true, italic: true, colors: paperBase },
  {
    id: 'studio', name: 'Studio Dark',
    colors: makeTheme('#1e1f22', '#2b2d30', '#393b40', '#43454a', '#1e1f22', '#4e5157', '#dfe1e5', '#9da0a8', '#6f737a', '#3574f0', '#214283', '#4b5059', '#26282e', jbSyntax, {}),
  },
  {
    id: 'studio-light', name: 'Studio Light',
    colors: { ...makeTheme('#ffffff', '#f7f8fa', '#dfe1e5', '#d5e1ff', '#ebecf0', '#c9ccd6', '#1e1f22', '#6c707e', '#a0a4b0', '#3574f0', '#a6d2ff', '#aeb3c2', '#f5f8fe', { code: '#080808', x: '#b85c00', k: '#0033b3', t: '#0033b3', y: '#008080', f: '#00627a', v: '#871094', o: '#008080', r: '#067d17', n: '#1750eb', c: '#8c8c8c', d: '#9e880d', w: '#9e880d', err: '#e53e4d', warn: '#c78a00', bp: '#e53e4d', 'git-add': '#55a76a', 'git-mod': '#3574f0' }, {}), glow: '#000000' },
  },
  {
    id: 'vivid-dark', name: 'Vivid Dark',
    colors: makeTheme('#1e1e1e', '#262626', '#333333', '#3f3f3f', '#191919', '#4a4a4a', '#dadada', '#b3b3b3', '#7a7a7a', '#a882ff', '#3d3560', '#5e5e5e', '#262626', { code: '#b8b8b8', k: '#fa99cd', t: '#ff8a8e', y: '#e9973f', f: '#e0de71', v: '#53dfdd', l: '#7ab8ff', o: '#53dfdd', r: '#44cf6e', n: '#a882ff', z: '#a882ff', c: '#868686', d: '#ff8a8e', w: '#e9973f', x: '#e9973f', err: '#fb464c', warn: '#e0de71', bp: '#fb464c', 'git-add': '#44cf6e', 'git-mod': '#7ab8ff' }, {}),
  },
  {
    id: 'vivid-light', name: 'Vivid Light',
    colors: { ...makeTheme('#ffffff', '#f6f6f6', '#e8e8e8', '#e3dcfa', '#e3e3e3', '#c8c8c8', '#222222', '#5c5c5c', '#8a8a8a', '#7852ee', '#d9cffb', '#ababab', '#f6f3fe', { code: '#4a4a4a', k: '#c2307a', t: '#c9263b', y: '#a85400', f: '#7f5e00', v: '#00807d', l: '#086ddd', o: '#00807d', r: '#067d39', n: '#7852ee', z: '#7852ee', c: '#767676', d: '#c9263b', w: '#a85400', x: '#a85400', err: '#e93147', warn: '#a85400', bp: '#e93147', 'git-add': '#067d39', 'git-mod': '#086ddd' }, {}), glow: '#000000' },
  },
  {
    id: 'nord', name: 'Nord',
    colors: makeTheme('#2e3440', '#272c36', '#3b4252', '#434c5e', '#232831', '#4c566a', '#eceff4', '#a5b0c2', '#6b768a', '#88c0d0', '#434c5e', '#4c566a', '#333a47', { code: '#d8dee9', x: '#d08770', k: '#81a1c1', t: '#81a1c1', y: '#8fbcbb', f: '#88c0d0', v: '#b48ead', o: '#8fbcbb', r: '#a3be8c', n: '#b48ead', c: '#616e88', d: '#5e81ac', w: '#d08770', err: '#bf616a', warn: '#ebcb8b', bp: '#bf616a', 'git-add': '#a3be8c', 'git-mod': '#81a1c1' }, {}),
  },
  {
    id: 'catppuccin', name: 'Catppuccin',
    colors: makeTheme('#1e1e2e', '#181825', '#313244', '#45475a', '#11111b', '#585b70', '#cdd6f4', '#a6adc8', '#6c7086', '#cba6f7', '#45475a', '#585b70', '#262637', { code: '#cdd6f4', x: '#fab387', k: '#cba6f7', t: '#f9e2af', y: '#f9e2af', f: '#89b4fa', v: '#f5c2e7', o: '#94e2d5', r: '#a6e3a1', n: '#fab387', c: '#6c7086', d: '#f5c2e7', w: '#fab387', err: '#f38ba8', warn: '#f9e2af', bp: '#f38ba8', 'git-add': '#a6e3a1', 'git-mod': '#89b4fa' }, {}),
  },
  {
    id: 'gruvbox', name: 'Gruvbox',
    colors: makeTheme('#282828', '#1d2021', '#3c3836', '#504945', '#1d2021', '#665c54', '#ebdbb2', '#bdae93', '#7c6f64', '#fabd2f', '#504945', '#665c54', '#32302f', { code: '#ebdbb2', x: '#fe8019', k: '#fb4934', t: '#fabd2f', y: '#fabd2f', f: '#b8bb26', v: '#83a598', o: '#8ec07c', r: '#b8bb26', n: '#d3869b', c: '#928374', d: '#8ec07c', w: '#fe8019', err: '#fb4934', warn: '#fabd2f', bp: '#fb4934', 'git-add': '#b8bb26', 'git-mod': '#83a598' }, {}),
  },
  {
    id: 'solarized', name: 'Solarized',
    colors: makeTheme('#002b36', '#00232c', '#073642', '#0b4452', '#001e26', '#2a525e', '#93a1a1', '#839496', '#586e75', '#268bd2', '#0b4452', '#2a525e', '#04323e', { code: '#93a1a1', x: '#cb4b16', k: '#859900', t: '#b58900', y: '#b58900', f: '#268bd2', v: '#d33682', o: '#2aa198', r: '#2aa198', n: '#d33682', c: '#586e75', d: '#dc322f', w: '#cb4b16', err: '#dc322f', warn: '#b58900', bp: '#dc322f', 'git-add': '#859900', 'git-mod': '#268bd2' }, {}),
  },
  {
    id: 'contrast', name: 'High Contrast', bold: true,
    colors: makeTheme('#000000', '#000000', '#2a2a2a', '#3d3d3d', '#6e6e6e', '#a0a0a0', '#ffffff', '#e0e0e0', '#bdbdbd', '#ffe600', '#5a4b00', '#bdbdbd', '#161616', { code: '#ffffff', x: '#ffb37f', k: '#ffe600', t: '#ffe600', y: '#7fe9ff', f: '#9dd8ff', v: '#ffb3f2', o: '#7fe9ff', r: '#8dff8d', n: '#ffc77f', c: '#bdbdbd', d: '#ffb37f', w: '#ffb37f', err: '#ff5c5c', warn: '#ffe600', bp: '#ff5c5c', 'git-add': '#8dff8d', 'git-mod': '#7fbfff' }, {}),
  },
]

function migrate() {
  const legacyLight = load('theme', 'dark') === 'light'
  const legacyColor = load('syntax', 'mono') === 'color'
  return { themeId: legacyColor ? (legacyLight ? 'studio-light' : 'black-color') : legacyLight ? 'paper' : 'black', edSize: load('font', 13), glow: load('glow', 'auto') }
}

export const DEFAULT_LOOK = {
  themeId: 'black', overrides: {}, bold: null, italic: null,
  uiFont: 'inter', uiSize: 12.5, radius: 4, density: 'normal',
  edFont: 'jetbrains', edCustom: '', edSize: 13, lineHeight: 1.55, caret: 'bar', rightMargin: 0, guides: true, nums: true, curline: true,
  glow: 'auto', glowStrength: 3, glowRadius: 200,
  fxUi: 'auto', fxCaret: 'auto', fxTrail: 'auto', fxType: 'auto',
}

export const look = { ...DEFAULT_LOOK, ...load('look', migrate()) }
export let customThemes = load('themes', [])

const allThemes = () => [...PRESETS, ...customThemes]
export const themes = allThemes
export const currentBase = () => allThemes().find((t) => t.id === look.themeId) ?? PRESETS[0]
export function colors() {
  const c = { ...blackBase, ...currentBase().colors, ...look.overrides }
  c.l = look.overrides.l ?? currentBase().colors.l ?? c.code
  c.z = look.overrides.z ?? currentBase().colors.z ?? c.k
  c.run = look.overrides.run ?? currentBase().colors.run ?? (luminance(c.bg) > 0.5 ? c.text : '#ffffff')
  return c
}
export const isCustomized = () => Object.keys(look.overrides).length > 0

const rgb = (c) => [1, 3, 5].map((i) => parseInt(c.slice(i, i + 2), 16))
const luminance = (c) => {
  const [r, g, b] = rgb(c)
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255
}

export function isColored() {
  const c = colors()
  return ['k', 'f', 'y', 'r'].some((key) => {
    const [r, g, b] = rgb(c[key])
    return Math.max(r, g, b) - Math.min(r, g, b) > 24
  })
}

const FONTS = {
  inter: "'Inter Tight Variable', system-ui, sans-serif",
  system: "system-ui, -apple-system, 'Segoe UI', sans-serif",
}
const MONOS = {
  jetbrains: "'JetBrains Mono Variable', 'DejaVu Sans Mono', monospace",
  system: 'monospace',
}
const DENSITY = { compact: { row: 20, bar: 30, tab: 26 }, normal: { row: 22, bar: 34, tab: 30 }, comfortable: { row: 26, bar: 38, tab: 34 } }

export function apply() {
  const s = root.style
  const c = colors()
  for (const [key, value] of Object.entries(c)) if (hex.test(value)) s.setProperty('--' + key, value)
  const light = luminance(c.bg) > 0.5
  s.colorScheme = light ? 'light' : 'dark'
  s.setProperty('--accent-text', luminance(c.accent) > 0.55 ? '#000000' : '#ffffff')
  s.setProperty('--shadow', light ? 'rgba(0,0,0,.16)' : 'rgba(0,0,0,.5)')
  s.setProperty('--scrim', light ? 'rgba(0,0,0,.12)' : 'rgba(0,0,0,.4)')
  const base = currentBase()
  s.setProperty('--kw', (look.bold ?? base.bold ?? false) ? '600' : '400')
  s.setProperty('--cs', (look.italic ?? base.italic ?? false) ? 'italic' : 'normal')
  s.setProperty('--sans', FONTS[look.uiFont] ?? FONTS.inter)
  s.setProperty('--ui-fs', look.uiSize + 'px')
  s.setProperty('--r1', look.radius + 'px')
  s.setProperty('--r2', Math.round(look.radius * 1.5) + 'px')
  s.setProperty('--r3', Math.round(look.radius * 2.75) + 'px')
  s.setProperty('--run-text', luminance(c.run) > 0.55 ? '#000000' : '#ffffff')
  s.setProperty('--cols', String(look.rightMargin || 120))
  root.dataset.margin = look.rightMargin ? 'on' : 'off'
  const d = DENSITY[look.density] ?? DENSITY.normal
  s.setProperty('--row', d.row + 'px')
  s.setProperty('--bar', d.bar + 'px')
  s.setProperty('--tab', d.tab + 'px')
  const custom = look.edFont === 'custom' && look.edCustom ? `"${look.edCustom.replace(/["\\]/g, '')}", monospace` : null
  s.setProperty('--mono', custom ?? MONOS[look.edFont] ?? MONOS.jetbrains)
  s.setProperty('--fs', look.edSize + 'px')
  s.setProperty('--lh', Math.round(look.edSize * look.lineHeight) + 'px')
  const [gr, gg, gb] = rgb(c.glow)
  s.setProperty('--glow-c', `rgba(${gr},${gg},${gb},${look.glowStrength / 100})`)
  s.setProperty('--glow-size', look.glowRadius + 'px')
  const capable = state.sys.cores >= 4 && (state.sys.total_mb === 0 || state.sys.total_mb >= 3500) && !matchMedia('(prefers-reduced-motion: reduce)').matches
  const effect = (v) => (v === 'on' || (v === 'auto' && capable) ? 'on' : 'off')
  root.dataset.fxUi = effect(look.fxUi)
  root.dataset.fxCaret = effect(look.fxCaret)
  root.dataset.fxTrail = effect(look.fxTrail)
  root.dataset.fxType = effect(look.fxType)
  root.dataset.caret = look.caret
  root.dataset.guides = look.guides ? 'on' : 'off'
  root.dataset.nums = look.nums ? 'on' : 'off'
  root.dataset.curline = look.curline ? 'on' : 'off'
}

let saveTimer = 0
export function save() {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(() => store('look', look), 200)
}

export function setTheme(id) {
  look.themeId = id
  look.overrides = {}
  look.bold = null
  look.italic = null
  save()
}

export function setColor(key, value) {
  if (!hex.test(value)) return
  look.overrides[key] = value.toLowerCase()
  save()
}

export function resetColors() {
  look.overrides = {}
  save()
}

export function resetAll() {
  Object.assign(look, DEFAULT_LOOK, { overrides: {} })
  save()
}

export function saveCustom(name) {
  const id = 'custom-' + Date.now().toString(36)
  customThemes = [...customThemes, { id, name, custom: true, colors: colors(), bold: look.bold ?? currentBase().bold, italic: look.italic ?? currentBase().italic }]
  store('themes', customThemes)
  setTheme(id)
}

export function deleteCustom(id) {
  customThemes = customThemes.filter((t) => t.id !== id)
  store('themes', customThemes)
  if (look.themeId === id) setTheme('black')
}

export function exportTheme() {
  const b = currentBase()
  return JSON.stringify({ name: b.name + (isCustomized() ? ' (custom)' : ''), colors: colors(), bold: look.bold ?? b.bold ?? false, italic: look.italic ?? b.italic ?? false }, null, 2)
}

export function importTheme(text) {
  const data = JSON.parse(text)
  const known = new Set(TOKENS.flatMap(([, list]) => list.map(([k]) => k)))
  const clean = {}
  for (const [k, v] of Object.entries(data.colors ?? {})) if (known.has(k) && hex.test(v)) clean[k] = v.toLowerCase()
  if (!Object.keys(clean).length) throw new Error('This is not a clight theme')
  const id = 'custom-' + Date.now().toString(36)
  customThemes = [...customThemes, { id, name: String(data.name ?? 'Imported').slice(0, 40), custom: true, colors: { ...blackBase, ...clean }, bold: !!data.bold, italic: !!data.italic }]
  store('themes', customThemes)
  setTheme(id)
}
