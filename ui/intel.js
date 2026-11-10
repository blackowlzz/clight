import * as ed from './editor.js'
import { $, h, invoke, listen, app, state, settings, reg, toast, guard, promptBox, palette, showMenu, basename, SEP, runAction } from './core.js'

const compEl = $('completion'), hoverEl = $('hover'), sigEl = $('sig')

export function setLsp(s) {
  state.lsp = s
  $('st-clangd').textContent = { off: 'clangd off', starting: 'clangd starting…', on: 'clangd', error: SEP === '\\' ? 'Install clangd…' : 'clangd unavailable' }[s]
}

export async function startIntel() {
  state.sys = await invoke('sys_info')
  const auto = state.sys.avail_mb === 0 || state.sys.avail_mb >= 1200
  if (settings.intel === 'off' || (settings.intel === 'auto' && !auto)) {
    setLsp('off')
    return
  }
  await restartIntel()
}

export async function restartIntel() {
  setLsp('starting')
  const index = settings.index === 'on' || (settings.index === 'auto' && state.sys.avail_mb >= 3000)
  try {
    await invoke('lsp_start', { root: state.root, build: state.build, index })
    setLsp('on')
    if (state.active !== null) ed.repaint()
    app.refreshStructure?.()
  } catch (e) {
    setLsp('error')
    console.warn(e)
    if (String(e).includes('not installed') && !warnedNoClangd && SEP !== '\\') {
      warnedNoClangd = true
      toast('clangd not found. Errors come from the compiler until you install LLVM')
    }
    state.tabs.filter((t) => !t.virtual).forEach((t) => syncDoc(t.id))
  }
}

let warnedNoClangd = false
let checking = false
let recheck = null

async function compilerCheck(id) {
  if (checking) {
    recheck = id
    return
  }
  checking = true
  await invoke('syntax_check', { id }).catch(() => {})
  checking = false
  if (recheck !== null) {
    const next = recheck
    recheck = null
    compilerCheck(next)
  }
}

export function syncDoc(id) {
  if (state.lsp === 'on') invoke('lsp_sync', { id }).catch(() => {})
  else if (state.lsp === 'error') compilerCheck(id)
}

export async function stopIntel() {
  await invoke('lsp_stop')
  setLsp('off')
  ed.repaint()
}

listen('lsp-exit', () => {
  if (state.lsp === 'on') setLsp('error')
})

$('st-clangd').addEventListener('click', async () => {
  if (state.lsp === 'on') await stopIntel()
  else if (state.lsp === 'error' && SEP === '\\') runAction('download-tools')
  else if (state.lsp !== 'starting' && state.root) await restartIntel()
})

const needLsp = () => {
  if (state.active === null) return false
  if (state.lsp !== 'on') {
    toast('clangd is off')
    return false
  }
  return true
}

const comp = { open: false, items: [], shown: [], sel: 0, prefix: '', token: 0, typed: '', incomplete: false }
let compTimer = 0
let sigTimer = 0
let hoverSticky = false

let ghost = null
let viewKey = ''
let ghostTimer = 0
let ghostToken = 0

function clearGhost() {
  ghostToken++
  clearTimeout(ghostTimer)
  if (!ghost) return
  ghost = null
  ed.setGhost('')
}
app.clearGhost = clearGhost

export function viewChanged(v) {
  viewKey = `${v.rev}:${v.line}:${v.col}`
  if (!ghost || ghost.key === viewKey) return
  // The caret moved along the suggestion because the typed letters matched it: keep it.
  if (ghost.base !== undefined && v.line === ghost.line && v.col >= ghost.base && v.col <= ghost.col && v.rev >= ghost.rev) {
    ghost.rev = v.rev
    ghost.base = v.col
    if (v.col === ghost.col) ghost.key = viewKey
    return
  }
  clearGhost()
}

async function requestGhost() {
  if (!settings.ghost || state.active === null || comp.open) return
  const token = ++ghostToken
  await ed.idle()
  const r = await invoke('line_suggest', { id: state.active, root: state.root ?? '' }).catch(() => null)
  if (token !== ghostToken || !r || comp.open || r.key !== viewKey) return
  const [rev, line, col] = r.key.split(':').map(Number)
  ghost = { ...r, rev, line, col, base: undefined }
  ed.setGhost(r.text)
}

// Typing these can add a closing character of its own, which would change the rest of the line.
const PAIRS = /[()[\]{}"'<>]/

/** Typed text that matches the start of the suggestion eats into it, so it stays on screen. */
export function typed(s = '') {
  clearTimeout(ghostTimer)
  if (ghost && s && !PAIRS.test(s) && ghost.text.length > s.length && ghost.text.startsWith(s)) {
    const n = [...s].length
    ghost.text = ghost.text.slice(s.length)
    ghost.base ??= ghost.col
    ghost.col += n
    ed.setGhost(ghost.text, ghost.col - ghost.base)
    return
  }
  ghostTimer = setTimeout(requestGhost, 60)
}

function acceptGhost(all) {
  const text = all ? ghost.text : (ghost.text.match(/^\s*\S+/)?.[0] ?? ghost.text)
  clearGhost()
  ed.send({ t: 'insert', s: text })
  typed()
}

export function hidePopups() {
  clearGhost()
  comp.open = false
  comp.token++
  clearTimeout(compTimer)
  compEl.hidden = true
  hoverEl.hidden = true
  hoverSticky = false
  sigEl.hidden = true
}

function place(el) {
  const c = ed.caretBox()
  const box = $('ed').getBoundingClientRect()
  const below = c.y + c.lh + el.offsetHeight + 4 < box.height
  el.style.left = Math.max(6, Math.min(c.x, box.width - el.offsetWidth - 10)) + 'px'
  el.style.top = (below ? c.y + c.lh + 2 : Math.max(0, c.y - el.offsetHeight - 2)) + 'px'
}

function renderComp() {
  const p = comp.prefix.toLowerCase()
  comp.shown = comp.items.filter((i) => i.insert.toLowerCase().startsWith(p)).slice(0, 40)
  if (!comp.shown.length) {
    comp.open = false
    compEl.hidden = true
    return
  }
  comp.open = true
  clearGhost()
  comp.sel = Math.min(comp.sel, comp.shown.length - 1)
  compEl.replaceChildren(...comp.shown.map((it, i) => h('div', { class: 'ci' + (i === comp.sel ? ' on' : ''), onpointerdown: (e) => (e.preventDefault(), acceptComp(i)) }, h('span', { class: 'lab' }, it.label), h('span', { class: 'det' }, it.detail))))
  compEl.hidden = false
  place(compEl)
}

function moveComp(d) {
  comp.sel = (comp.sel + d + comp.shown.length) % comp.shown.length
  ;[...compEl.children].forEach((c, i) => c.classList.toggle('on', i === comp.sel))
  compEl.children[comp.sel].scrollIntoView({ block: 'nearest' })
}

function acceptComp(i) {
  const it = comp.shown[i]
  hidePopups()
  if (it) {
    ed.send({ t: 'complete', s: it.insert })
    if (it.insert.endsWith('(')) requestSig()
  }
}

async function requestComp(force) {
  const id = state.active
  if (id === null) return
  const token = ++comp.token
  comp.typed = ''
  await ed.idle()
  try {
    const r = await invoke(state.lsp === 'on' ? 'lsp_complete' : 'word_complete', { id, force })
    if (token !== comp.token) return
    comp.items = r.items
    comp.incomplete = r.incomplete
    comp.prefix = r.prefix + comp.typed
    comp.sel = 0
    if (r.items.length) renderComp()
  } catch (e) {
    console.warn(e)
  }
}

async function requestSig() {
  const id = state.active
  if (id === null || state.lsp !== 'on') return
  await ed.idle()
  try {
    const s = await invoke('lsp_signature', { id })
    if (!s) {
      sigEl.hidden = true
      return
    }
    const [label, range] = s
    if (range) sigEl.replaceChildren(label.slice(0, range[0]), h('b', {}, label.slice(range[0], range[1])), label.slice(range[1]))
    else sigEl.textContent = label
    sigEl.hidden = false
    const c = ed.caretBox()
    sigEl.style.left = Math.max(6, c.x - 20) + 'px'
    sigEl.style.top = Math.max(0, c.y - sigEl.offsetHeight - 3) + 'px'
  } catch (e) {
    console.warn(e)
  }
}

const isWordChar = (s) => s.length === 1 && /[\p{L}\p{N}_]/u.test(s)

export function beforeKey(e, name) {
  if (name === 'Escape' && (!hoverEl.hidden || !sigEl.hidden || comp.open)) {
    hidePopups()
    return true
  }
  if (comp.open) {
    if (name === 'ArrowDown') return moveComp(1), true
    if (name === 'ArrowUp') return moveComp(-1), true
    if (name === 'Enter' || name === 'Tab') return acceptComp(comp.sel), true
  }
  if (ghost) {
    if (name === 'Tab') return acceptGhost(true), true
    if (name === 'C-ArrowRight') return acceptGhost(false), true
    if (name === 'Escape') return clearGhost(), true
  }
  if (name === 'C- ') {
    requestComp(true)
    return true
  }
  return false
}

export function afterType(s) {
  hoverEl.hidden = true
  if (s === '(' || s === ',') {
    clearTimeout(sigTimer)
    sigTimer = setTimeout(requestSig, 120)
  } else if (s === ')' || s === ';') sigEl.hidden = true

  if (isWordChar(s)) {
    comp.typed += s
    if (comp.open) {
      comp.prefix += s
      renderComp()
      if (comp.incomplete || !comp.open) {
        clearTimeout(compTimer)
        compTimer = setTimeout(() => requestComp(false), 180)
      }
    } else {
      clearTimeout(compTimer)
      compTimer = setTimeout(() => requestComp(false), 180)
    }
  } else if (s === 'Backspace' && comp.open && comp.prefix.length > 1) {
    comp.prefix = comp.prefix.slice(0, -1)
    renderComp()
  } else {
    const wasOpen = comp.open
    comp.open = false
    comp.token++
    clearTimeout(compTimer)
    compEl.hidden = true
    if (!wasOpen && (s === '.' || s === '>' || s === ':')) compTimer = setTimeout(() => requestComp(false), 180)
  }
}

export function showHover(text, atCaret = true, point = null) {
  hoverEl.textContent = text
  hoverEl.hidden = false
  hoverSticky = atCaret
  if (atCaret) place(hoverEl)
  else {
    const box = $('ed').getBoundingClientRect()
    hoverEl.style.left = Math.max(6, Math.min(point.x - box.left, box.width - hoverEl.offsetWidth - 10)) + 'px'
    const y = point.y - box.top
    hoverEl.style.top = (y + hoverEl.offsetHeight + 24 < box.height ? y + 16 : Math.max(0, y - hoverEl.offsetHeight - 8)) + 'px'
  }
}

export function mouseHover(cell, point) {
  if (state.active === null || comp.open) return
  invoke('doc_probe', { id: state.active, row: cell.line, vcol: cell.vcol }).then(async ([diag, word]) => {
    if (diag) return showHover(diag, false, point)
    const value = word ? await app.debug?.evaluateHover?.(word) : null
    if (value) showHover(`${word} = ${value}`, false, point)
  })
}

export function hoverEnd() {
  if (!hoverSticky) hoverEl.hidden = true
}

export async function applyEdits(changes) {
  for (const c of changes) {
    const d = await invoke('doc_open', { path: c.path })
    await app.ensureTab(d)
    await invoke('doc_apply_edits', { id: d.id, edits: c.edits })
  }
  ed.repaint()
  app.markAllDirty?.()
}
listen('apply-edit', (e) => guard(applyEdits(e.payload)))

async function gotoDefinition() {
  if (!needLsp()) return
  await ed.idle()
  const loc = await invoke('lsp_definition', { id: state.active })
  if (loc) await app.jumpTo(loc.path, loc.line, loc.col)
  else toast('No definition found')
}

async function quickDoc() {
  if (!needLsp()) return
  await ed.idle()
  const s = await invoke('lsp_hover', { id: state.active })
  if (s) showHover(s)
  else toast('Nothing to show here')
}

async function format() {
  if (state.active === null) return
  await ed.idle()
  try {
    const changed = await invoke('doc_format', { id: state.active, style: settings.formatStyle })
    ed.repaint()
    if (!changed) toast('Already formatted')
  } catch (e) {
    if (String(e).includes('not installed') && state.lsp === 'on') {
      await invoke('lsp_format', { id: state.active })
      ed.repaint()
    } else toast(String(e).includes('not installed') ? 'Install clang-format to reformat code' : e)
  }
}

async function findUsages() {
  if (!needLsp()) return
  await ed.idle()
  const hits = await invoke('lsp_references', { id: state.active })
  app.showHits(`Usages`, `${hits.length} found`, hits)
}

async function rename() {
  if (!needLsp()) return
  await ed.idle()
  const v = ed.lastView()
  const [, word] = await invoke('doc_probe', { id: state.active, row: v.cur[0], vcol: v.cur[1] })
  if (!word) return toast('Place the caret on a symbol')
  const name = await promptBox(`Rename ${word}`, word, 'Rename')
  if (!name || name === word) return
  const changes = await invoke('lsp_rename', { id: state.active, name })
  await applyEdits(changes)
  toast(`Renamed in ${changes.length} file${changes.length === 1 ? '' : 's'}`)
}

async function codeActions() {
  if (!needLsp()) return
  await ed.idle()
  const list = await invoke('lsp_code_actions', { id: state.active })
  if (!list.length) return toast('No actions here')
  const c = ed.caretBox()
  const box = $('ed').getBoundingClientRect()
  showMenu(
    box.left + c.x,
    box.top + c.y + c.lh + 2,
    list.map((a) => [
      a.title,
      () => {
        if (a.edit.length) guard(applyEdits(a.edit))
        else if (a.command) guard(invoke('lsp_exec_command', { command: a.command }).then(() => ed.repaint()))
      },
    ]),
  )
}

async function switchHeader() {
  if (!needLsp()) return
  const path = await invoke('lsp_switch_header', { id: state.active })
  if (path) await app.openFile(path)
  else toast('No matching header or source file')
}

const KIND = { 5: 'cl', 6: 'm', 8: 'f', 9: 'c', 10: 'en', 12: 'fn', 13: 'v', 14: 'k', 22: 'e', 23: 'st', 3: 'ns' }

async function fileSymbols() {
  if (!needLsp()) return
  await ed.idle()
  const syms = await invoke('lsp_symbols', { id: state.active })
  palette({
    placeholder: 'Go to member',
    query: async (q) => syms.filter((s) => s.name.toLowerCase().includes(q.toLowerCase())).slice(0, 80).map((s) => ({ ...s, open: () => ed.send({ t: 'goto', line: s.line, col: s.col }) })),
    render: (s) => h('div', {}, h('b', {}, s.name), h('span', {}, s.detail), h('span', { class: 'kind' }, KIND[s.kind] ?? '')),
  })
}

function workspaceSymbols() {
  if (state.lsp !== 'on') return toast('clangd is off')
  palette({
    placeholder: 'Go to symbol',
    debounce: 200,
    query: async (q) => {
      if (q.length < 2) return []
      const hits = await invoke('lsp_workspace_symbols', { query: q })
      return hits.map((x) => ({ ...x, open: () => guard(app.jumpTo(x.path, x.line, x.col)) }))
    },
    render: (x) => h('div', {}, h('b', {}, x.text), h('span', {}, basename(x.path))),
  })
}

export function renderSymbols(el, syms) {
  el.replaceChildren(
    ...syms.map((s) => {
      const row = h('div', { class: 'row', onclick: () => ed.send({ t: 'goto', line: s.line, col: s.col }) }, h('span', { class: 'ty' }, KIND[s.kind] ?? ''), h('span', { class: 'nm' }, s.name))
      row.style.paddingLeft = 4 + s.depth * 12 + 'px'
      return row
    }),
  )
}

reg('goto-def', 'Go to Definition', 'C-b', gotoDefinition)
reg('goto-def-2', 'Go to Definition', 'F12', gotoDefinition, { hidden: true })
reg('quick-doc', 'Quick Documentation', 'C-q', quickDoc)
reg('format', 'Reformat Code', 'C-A-l', format)
reg('usages', 'Find Usages', 'A-F7', findUsages)
reg('rename', 'Rename Symbol…', 'S-F6', rename)
reg('code-actions', 'Show Quick Fixes', 'A-Enter', codeActions)
reg('switch-header', 'Switch Header/Source', 'A-o', switchHeader)
reg('file-symbols', 'File Structure', 'C-F12', fileSymbols)
reg('symbols', 'Go to Symbol…', 'C-A-S-n', workspaceSymbols)

app.gotoDefinition = () => guard(gotoDefinition())