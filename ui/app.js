import * as ed from './editor.js'
import { $, h, invoke, listen, win, root, app, state, settings, load, store, toast, guard, actions, reg, runAction, keyName, keyLabel, modal, modalOpen, closeModal, confirmBox, promptBox, palette, fuzzy, seg, showMenu, hideMenu, menuVisible, join, dirname, basename, SEP, activeTab, fileIcon, iconBtn, icon } from './core.js'
import * as intel from './intel.js'
import * as build from './build.js'
import * as git from './git.js'
import * as update from './update.js'
import * as tools from './tools.js'
import * as ai from './ai.js'
import { closeNote } from './notify.js'
import { TERMINAL_PASS } from './termkeys.js'
import * as theme from './theme.js'
import * as settingsPage from './settings.js'
import { renderHome, tile } from './welcome.js'

function applyLook() {
  theme.apply()
  applyGlow()
  ed.measure()
}
app.applyLook = () => {
  applyGlow()
  ed.measure()
}

const glow = $('glow')
let glowOn = false, glowX = 0, glowY = 0, glowQueued = false

function moveGlow() {
  glowQueued = false
  glow.style.transform = `translate(${glowX - theme.look.glowRadius / 2}px,${glowY - theme.look.glowRadius / 2}px)`
}
function onPointerGlow(e) {
  glowX = e.clientX
  glowY = e.clientY
  glow.hidden = false
  if (!glowQueued) {
    glowQueued = true
    requestAnimationFrame(moveGlow)
  }
}
function applyGlow() {
  const want = theme.look.glow === 'on' || (theme.look.glow === 'auto' && state.sys.cores >= 4 && (state.sys.total_mb === 0 || state.sys.total_mb >= 3500))
  if (want === glowOn) return
  glowOn = want
  glow.hidden = true
  document[want ? 'addEventListener' : 'removeEventListener']('pointermove', onPointerGlow, { passive: true })
}
document.documentElement.addEventListener('mouseleave', () => (glow.hidden = true))

let debugMod = null
let termMod = null

async function loadDebug() {
  debugMod ??= await import('./debug.js')
  app.debug = debugMod
  return debugMod
}

let chatMod = null
async function loadChat() {
  chatMod ??= await import('./chat.js')
  return chatMod
}

async function loadTerm() {
  termMod ??= await import('./terminal.js')
  app.term = termMod
  return termMod
}

const LEFT = { project: 'Project', structure: 'Structure', git: 'Commit' }
const BOTTOM = { build: 'Build', run: 'Run', debug: 'Debug', problems: 'Problems', find: 'Find', terminal: 'Terminal' }

function showLeft(name, force = false) {
  const same = state.left === name
  state.left = same && !force ? null : name
  $('left').hidden = state.left === null
  $('side-rz').hidden = state.left === null
  for (const n of Object.keys(LEFT)) $('v-' + n).hidden = n !== state.left
  document.querySelectorAll('[data-left]').forEach((b) => b.classList.toggle('on', b.dataset.left === state.left))
  if (state.left === null) {
    ed.focus()
  } else {
    $('left-title').textContent = LEFT[state.left]
    const acts = $('left-actions')
    acts.replaceChildren()
    if (state.left === 'structure') refreshStructure()
    if (state.left === 'git') {
      acts.append(git.panelActions())
      git.shown()
    }
  }
  saveSession()
}

async function showBottom(name, force = true) {
  if (state.bottom === name && !force) {
    hideBottom()
    return
  }
  state.bottom = name
  $('panel').hidden = false
  $('panel-rz').hidden = false
  for (const n of Object.keys(BOTTOM)) $('v-' + n).hidden = n !== name
  document.querySelectorAll('[data-bottom]').forEach((b) => b.classList.toggle('on', b.dataset.bottom === name))
  $('panel-title').textContent = BOTTOM[name]
  $('panel-sub').textContent = ''
  const acts = $('panel-actions')
  acts.replaceChildren()
  if (name === 'build' || name === 'run') {
    const out = name === 'build' ? build.buildOut : build.runOut
    if (name === 'run') acts.append(iconBtn('refresh', 'Rerun (Shift+F10)', () => guard(app.rerun())), iconBtn('stop', 'Stop (Ctrl+F2)', () => guard(app.stopTask())))
    acts.append(h('button', { class: 'plain', onclick: () => (name === 'run' ? build.clearRun() : out.clear()) }, 'Clear'))
    if (name === 'run') build.runTermShown()
  } else if (name === 'debug') {
    const d = await loadDebug()
    if (state.bottom === 'debug') {
      acts.append(d.panelActions())
      d.refreshHeader()
    }
  } else if (name === 'terminal') {
    const t = await loadTerm()
    if (state.bottom === 'terminal') {
      acts.append(t.panelActions())
      t.shown()
    }
  } else if (name === 'problems') refreshProblems()
  saveSession()
}

function hideBottom() {
  state.bottom = null
  $('panel').hidden = true
  $('panel-rz').hidden = true
  document.querySelectorAll('[data-bottom]').forEach((b) => b.classList.remove('on'))
  ed.focus()
  saveSession()
}

app.showLeft = showLeft
app.showBottom = showBottom
document.querySelectorAll('[data-left]').forEach((b) => b.addEventListener('click', () => showLeft(b.dataset.left)))
document.querySelectorAll('[data-bottom]').forEach((b) => b.addEventListener('click', () => showBottom(b.dataset.bottom, false)))
$('panel-x').addEventListener('click', hideBottom)

const recents = () => load('recent', [])

function folderBrowser(title, action, start, extra, onChoose) {
  let current = start
  const pathInput = h('input', { value: current, spellcheck: 'false' })
  const list = h('div', { class: 'browser' })
  async function go(dir) {
    try {
      const items = await invoke('list_dir', { path: dir, hidden: false })
      current = dir
      pathInput.value = dir
      const up = h('div', { class: 'row dir', onclick: () => go(dirname(dir)) }, h('span', { class: 'tw' }, '‹'), '..')
      list.replaceChildren(up, ...items.filter((e) => e.dir).map((e) => h('div', { class: 'row dir', onclick: () => go(e.path) }, h('span', { class: 'tw' }, '›'), e.name)))
      list.scrollTop = 0
    } catch (e) {
      toast(e)
    }
  }
  pathInput.addEventListener('keydown', (e) => e.key === 'Enter' && go(pathInput.value))
  let close
  close = modal(h('div', { class: 'dialog' }, h('h3', {}, title), pathInput, list, extra, h('div', { class: 'actions' }, h('button', { onclick: () => close() }, 'Cancel'), h('button', { class: 'primary', onclick: () => onChoose(current, close) }, action))))
  go(start)
}

function cloneRepo() {
  const url = h('input', { placeholder: 'https://github.com/user/repository.git', spellcheck: 'false' })
  folderBrowser('Clone repository', 'Clone', state.sys.home, url, async (dir, close) => {
    if (!url.value.trim()) return toast('Enter the repository URL')
    toast('Cloning…')
    try {
      const path = await invoke('git_clone', { url: url.value.trim(), parent: dir })
      close()
      await openProject(path)
    } catch (e) {
      toast(e)
    }
  })
  url.focus()
}
app.cloneRepo = cloneRepo

function pickFolder() {
  folderBrowser('Open folder', 'Open', recents()[0] ? dirname(recents()[0]) : state.sys.home, null, (dir, close) => {
    close()
    guard(openProject(dir))
  })
}

function newProject() {
  const name = h('input', { placeholder: 'Project name', spellcheck: 'false' })
  let cpp = true
  const extra = h(
    'div',
    { style: 'display:grid;gap:10px' },
    name,
    h('div', { class: 'row2' }, h('span', {}, 'Language'), seg([[true, 'C++'], [false, 'C']], true, (v) => (cpp = v))),
  )
  folderBrowser('New project', 'Create', state.sys.home, extra, async (dir, close) => {
    try {
      const path = await invoke('create_project', { parent: dir, name: name.value.trim(), cpp })
      close()
      await openProject(path)
      await openFile(join(join(path, 'src'), cpp ? 'main.cpp' : 'main.c'))
    } catch (e) {
      toast(e)
    }
  })
}

let restoring = false

app.pickFolder = pickFolder
app.newProject = newProject
app.openProject = (p) => openProject(p)
app.loadDebugActions = async () => {
  await loadDebug()
}

async function openProject(path) {
  await invoke('list_dir', { path, hidden: true })
  if (state.root) await closeProject()
  state.root = path
  state.files = null
  store('recent', [path, ...recents().filter((p) => p !== path)].slice(0, 20))
  store('recentTime', { ...load('recentTime', {}), [path]: Date.now() })
  const s = load('session:' + path, {})
  build.setProfile(s.profile ?? 'Debug')
  state.bps = new Map(Object.entries(s.bps ?? {}).map(([k, v]) => [k, new Set(v)]))
  state.conds = new Map(Object.entries(s.conds ?? {}))
  $('home').hidden = true
  $('work').hidden = false
  $('status').hidden = false
  $('run-tools').hidden = false
  renderProjectChip()
  renderMenus()
  expanded.clear()
  for (const p of s.expanded ?? []) expanded.add(p)
  dirCache.clear()
  await refreshTree()
  await build.refreshTargets()
  ed.measure()
  restoring = true
  state.left = null
  showLeft(s.left ?? 'project', true)
  git.refresh()
  await intel.startIntel()
  for (const p of s.tabs ?? []) {
    if (await invoke('path_exists', { path: p })) await openFile(p, false).catch(() => {})
  }
  const active = state.tabs.find((t) => t.path === s.active) ?? state.tabs[0]
  if (active) activate(active.id)
  if (s.cursors) cursors = s.cursors
  for (const t of state.tabs) {
    const c = cursors[t.path]
    if (c) ed.docCommand(t.id, [{ t: 'goto', line: c[0], col: c[1] }])
  }
  restoring = false
  if (state.active !== null) ed.send()
  renderEmpty()
}

async function closeProject() {
  await saveAll()
  saveSession()
  for (const t of [...state.tabs]) {
    ed.forget(t.id)
    await invoke('doc_close', { id: t.id })
  }
  state.tabs = []
  state.active = null
  ed.show(null)
  renderTabs()
  await intel.stopIntel()
  termMod?.closeAll()
}

let cursors = {}
let sessionTimer = 0

function sessionSnapshot() {
  return {
    tabs: state.tabs.filter((t) => !t.virtual).map((t) => t.path),
    active: activeTab()?.path,
    expanded: [...expanded],
    left: state.left,
    bottom: state.bottom,
    profile: state.profile,
    cursors,
    bps: Object.fromEntries([...state.bps].filter(([, s]) => s.size).map(([k, s]) => [k, [...s]])),
    conds: Object.fromEntries(state.conds),
  }
}

function saveSession() {
  if (restoring || !state.root) return
  clearTimeout(sessionTimer)
  sessionTimer = setTimeout(() => store('session:' + state.root, sessionSnapshot()), 300)
}
app.saveSession = saveSession

function renderEmpty() {
  const row = (label, k) => [h('span', {}, label), h('kbd', {}, k)]
  $('empty').replaceChildren(h('div', { class: 'keys' }, ...row('Open file', 'Ctrl P'), ...row('Find in project', 'Ctrl Shift F'), ...row('Every command', 'Ctrl Shift A'), ...row('Terminal', 'Alt F12')))
}

const expanded = new Set()
const dirCache = new Map()
const tree = $('v-project')

async function loadDir(path) {
  const items = await invoke('list_dir', { path, hidden: false })
  dirCache.set(path, items)
  return items
}

async function refreshTree() {
  dirCache.clear()
  await loadDir(state.root)
  await Promise.all([...expanded].map((p) => loadDir(p).catch(() => expanded.delete(p))))
  renderTree()
}

function renderTree() {
  if (!state.root) return
  const rows = []
  const activePath = activeTab()?.path
  const walk = (path, depth) => {
    for (const e of dirCache.get(path) ?? []) {
      const open = e.dir && expanded.has(e.path)
      const st = git.fileStatus(e.path)
      const row = document.createElement('div')
      row.className = 'row' + (e.dir ? ' dir' : '') + (open ? ' open' : '') + (e.path === activePath ? ' on' : '') + (st ? ' mod' : '')
      row.style.paddingLeft = 4 + depth * 12 + 'px'
      row.dataset.path = e.path
      if (e.dir) row.dataset.dir = '1'
      const tw = document.createElement('span')
      tw.className = 'tw'
      tw.textContent = e.dir ? '›' : ''
      const nm = document.createElement('span')
      nm.className = 'nm'
      nm.textContent = e.name
      row.append(tw, fileIcon(e.name, e.dir), nm)
      rows.push(row)
      if (open) walk(e.path, depth + 1)
    }
  }
  const home = state.sys.home
  const rootRow = h('div', { class: 'row root' }, h('span', { class: 'tw' }), fileIcon('', true), h('span', {}, basename(state.root)), h('span', { class: 'rootpath' }, home && state.root.startsWith(home) ? '~' + state.root.slice(home.length) : state.root))
  rows.push(rootRow)
  walk(state.root, 1)
  tree.replaceChildren(...rows)
}
app.renderTree = renderTree

tree.addEventListener('click', async (e) => {
  const row = e.target.closest('.row')
  if (!row) return
  const path = row.dataset.path
  if (row.dataset.dir) {
    if (expanded.has(path)) expanded.delete(path)
    else {
      await guard(loadDir(path))
      expanded.add(path)
    }
    renderTree()
    saveSession()
  } else guard(openFile(path))
})

tree.addEventListener('contextmenu', (e) => {
  e.preventDefault()
  const row = e.target.closest('.row')
  const path = row?.dataset.path ?? state.root
  const isDir = !row || !!row.dataset.dir
  const dir = isDir ? path : dirname(path)
  const items = [
    ['New file…', () => createEntry(dir, false)],
    ['New folder…', () => createEntry(dir, true)],
  ]
  if (row) items.push('-', ['Rename…', () => renameEntry(path)], ['Delete…', () => deleteEntry(path)])
  showMenu(e.clientX, e.clientY, items)
})

async function afterFsChange(dir) {
  if (dir !== state.root) expanded.add(dir)
  state.files = null
  await refreshTree()
  git.refresh()
}

async function createEntry(dir, isDir) {
  const name = await promptBox(isDir ? 'New folder' : 'New file', '', 'Create')
  if (!name) return
  const path = join(dir, name)
  try {
    await invoke(isDir ? 'fs_new_dir' : 'fs_new_file', { path })
    await afterFsChange(dir)
    if (!isDir) await openFile(path)
  } catch (e) {
    toast(e)
  }
}

async function renameEntry(path) {
  const name = await promptBox('Rename', basename(path), 'Rename')
  if (!name || name === basename(path)) return
  const open = state.tabs.find((t) => t.path === path)
  if (open?.dirty) return toast('Save the file before renaming it')
  try {
    await invoke('fs_rename', { from: path, to: join(dirname(path), name) })
    if (open) {
      await forceCloseTab(open)
      await openFile(join(dirname(path), name))
    }
    await afterFsChange(dirname(path))
  } catch (e) {
    toast(e)
  }
}

async function deleteEntry(path) {
  const yes = await confirmBox(`Delete ${basename(path)}?`, 'This removes it from disk and cannot be undone.', 'Delete')
  if (!yes) return
  try {
    for (const t of state.tabs.filter((t) => t.path === path || t.path.startsWith(path + SEP))) await forceCloseTab(t)
    await invoke('fs_delete', { path })
    expanded.delete(path)
    state.files = null
    await refreshTree()
    git.refresh()
  } catch (e) {
    toast(e)
  }
}

function renderTabs() {
  $('tabs').replaceChildren(
    ...state.tabs.map((t) =>
      h(
        'div',
        {
          class: 'tab' + (t.id === state.active ? ' on' : '') + (t.dirty ? ' dirty' : ''),
          title: t.path,
          onpointerdown: (e) => {
            if (e.button === 1) {
              e.preventDefault()
              closeTab(t)
            } else if (e.button === 0 && !e.target.closest('.dot')) {
              navPush()
              activate(t.id)
            }
          },
          oncontextmenu: (e) => {
            e.preventDefault()
            showMenu(e.clientX, e.clientY, [
              ['Close', () => closeTab(t), 'Ctrl+F4'],
              ['Close Others', () => closeOthers(t)],
              ['Close All', () => closeOthers(null)],
              '-',
              ['Copy Path', () => copyText(t.path)],
              ['Show in Project', () => reveal(t.path), '', !t.virtual],
            ])
          },
        },
        fileIcon(t.name),
        t.name,
        h('span', { class: 'dot', onclick: () => closeTab(t) }, h('i', {}, '×')),
      ),
    ),
  )
  $('tabs').querySelector('.on')?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  renderPath()
}

function renderPath() {
  const t = activeTab()
  const el = $('st-path')
  if (!t || t.virtual || !state.root || !t.path.startsWith(state.root)) {
    el.replaceChildren()
    return
  }
  const parts = [basename(state.root), ...t.path.slice(state.root.length + 1).split(SEP)]
  el.replaceChildren(...parts.flatMap((p, i) => (i === parts.length - 1 ? [h('b', {}, p)] : [p, h('i', {}, '›')])))
}

function renderProjectChip() {
  const chip = $('proj-chip')
  chip.replaceChildren(tile(basename(state.root)), h('span', {}, basename(state.root)), icon('chevron'))
  chip.hidden = false
}

function projectMenu() {
  const r = $('proj-chip').getBoundingClientRect()
  const others = recents().filter((p) => p !== state.root).slice(0, 8)
  showMenu(r.left, r.bottom + 2, [
    ...others.map((p) => [basename(p), () => guard(openProject(p))]),
    ...(others.length ? ['-'] : []),
    ['Open Folder…', pickFolder],
    ['New Project…', newProject],
    ['Close Project', () => runAction('close-project')],
  ])
}
$('proj-chip').addEventListener('click', projectMenu)
$('gear').addEventListener('click', () => settingsPage.open())

function copyText(text) {
  const fn = (e) => {
    e.clipboardData.setData('text/plain', text)
    e.preventDefault()
  }
  document.addEventListener('copy', fn, { once: true, capture: true })
  document.execCommand('copy')
  document.removeEventListener('copy', fn, { capture: true })
}

app.copyText = copyText

async function reveal(path) {
  showLeft('project', true)
  let dir = dirname(path)
  const chain = []
  while (dir.startsWith(state.root) && dir !== state.root) {
    chain.unshift(dir)
    dir = dirname(dir)
  }
  for (const d of chain) {
    expanded.add(d)
    await loadDir(d)
  }
  renderTree()
  tree.querySelector('.on')?.scrollIntoView({ block: 'nearest' })
}

function activate(id) {
  if (state.active === id) {
    ed.focus()
    return
  }
  intel.hidePopups()
  state.active = id
  ed.show(id)
  renderTabs()
  renderTree()
  if (findOpen) runSearch()
  if (state.left === 'structure') refreshStructure()
  const t = activeTab()
  if (t && !t.virtual) {
    recent = [t.path, ...recent.filter((p) => p !== t.path)].slice(0, 30)
    invoke('git_refresh', { id }).then(() => ed.repaint())
  }
  saveSession()
}

async function ensureTab(d) {
  let tab = state.tabs.find((t) => t.id === d.id)
  if (!tab) {
    tab = { ...d, dirty: false }
    state.tabs.push(tab)
    const lines = state.bps.get(d.path)
    if (lines?.size) await invoke('doc_apply', { id: d.id, cmds: [{ t: 'set_breakpoints', lines: [...lines] }], top: 0, rows: 1, reveal: false })
    intel.syncDoc(d.id)
    renderTabs()
  }
  return tab
}

async function openFile(path, focus = true) {
  const d = await invoke('doc_open', { path })
  const tab = await ensureTab(d)
  if (focus) activate(d.id)
  return tab
}

async function openVirtual(key, name, text) {
  const d = await invoke('doc_open_virtual', { key, name, text })
  let tab = state.tabs.find((t) => t.id === d.id)
  if (tab) {
    if (state.active === d.id) ed.repaint()
  } else {
    tab = { ...d, dirty: false, virtual: true }
    state.tabs.push(tab)
  }
  activate(d.id)
  renderTabs()
}

const nav = { back: [], forward: [] }

function navPush() {
  const v = ed.lastView()
  const t = activeTab()
  if (!v || !t || t.virtual) return
  const last = nav.back[nav.back.length - 1]
  if (last && last.path === t.path && last.line === v.line) return
  nav.back.push({ path: t.path, line: v.line, col: v.col - 1 })
  if (nav.back.length > 60) nav.back.shift()
  nav.forward.length = 0
}

function navGo(from, to) {
  const v = ed.lastView()
  const t = activeTab()
  const target = from.pop()
  if (!target) return
  if (v && t && !t.virtual) to.push({ path: t.path, line: v.line, col: v.col - 1 })
  guard(placeCaret(target.path, target.line, target.col))
}

async function placeCaret(path, line, col) {
  await openFile(path)
  await ed.idle()
  ed.send({ t: 'goto', line, col })
}

async function jumpTo(path, line, col) {
  navPush()
  await placeCaret(path, line, col)
}

async function forceCloseTab(t) {
  const i = state.tabs.indexOf(t)
  state.tabs.splice(i, 1)
  ed.forget(t.id)
  await invoke('doc_close', { id: t.id })
  if (state.active === t.id) {
    state.active = null
    const next = state.tabs[Math.min(i, state.tabs.length - 1)]
    if (next) activate(next.id)
    else {
      ed.show(null)
      setStatusEmpty()
    }
  }
  renderTabs()
  renderTree()
  saveSession()
}

async function closeTab(t) {
  if (t.dirty) {
    const save = await confirmBox(`Save ${t.name}?`, 'Your changes will be lost if you close without saving.', 'Save', 'Discard')
    if (save && !(await saveTab(t))) return
  }
  await forceCloseTab(t)
}

async function closeOthers(keep) {
  for (const t of state.tabs.filter((x) => x !== keep)) await closeTab(t)
}

let gitTimer = 0

async function saveTab(t, { force = false, quiet = false } = {}) {
  if (t.virtual) return true
  await ed.idle()
  try {
    const ok = await invoke('doc_save', { id: t.id, force })
    if (!ok) {
      if (quiet) return false
      const yes = await confirmBox(`${t.name} changed on disk`, 'Another program modified this file. Overwrite it with your version?', 'Overwrite')
      return yes ? saveTab(t, { force: true }) : false
    }
    t.dirty = false
    renderTabs()
    if (t.id === state.active) ed.flashSaved()
    await invoke('git_refresh', { id: t.id })
    if (t.id === state.active) ed.repaint()
    clearTimeout(gitTimer)
    gitTimer = setTimeout(() => git.refresh(), 300)
    if (state.left === 'structure' && t.id === state.active) refreshStructure()
    return true
  } catch (e) {
    toast(e)
    return false
  }
}

async function saveAll(quiet = false) {
  await ed.idle()
  await syncDirty()
  for (const t of state.tabs.filter((t) => t.dirty)) if (!(await saveTab(t, { quiet })) && !quiet) return false
  return true
}
app.saveAll = saveAll

async function syncDirty() {
  for (const t of state.tabs) {
    if (t.id === state.active) continue
    const v = await invoke('doc_apply', { id: t.id, cmds: [], top: 0, rows: 1, reveal: false }).catch(() => null)
    if (v) t.dirty = v.dirty
  }
  renderTabs()
}

app.markAllDirty = () => guard(syncDirty())
app.openFile = openFile
app.openVirtual = openVirtual
app.jumpTo = jumpTo
app.ensureTab = ensureTab
app.restartIntel = () => guard(intel.restartIntel())
app.refocus = () => {
  if (!$('work').hidden && ed.current() !== null) ed.focus()
}
app.afterExternalChange = async () => {
  const changed = await invoke('docs_refresh')
  if (changed.includes(state.active)) ed.repaint()
  await refreshTree()
  for (const t of state.tabs) if (!t.virtual) invoke('git_refresh', { id: t.id }).then(() => t.id === state.active && ed.repaint())
}

let lspTimer = 0
const lastRev = new Map()

function setStatusEmpty() {
  for (const id of ['st-msg', 'st-pos', 'st-diag', 'st-indent', 'st-eol', 'st-path']) $(id).textContent = ''
}

ed.configure({
  onChange(id, v) {
    intel.viewChanged(v)
    const tab = state.tabs.find((t) => t.id === id)
    if (!tab) return
    if (tab.dirty !== v.dirty) {
      tab.dirty = v.dirty
      renderTabs()
    }
    $('st-pos').textContent = `${v.line + 1}:${v.col}`
    $('st-indent').textContent = v.readonly ? 'Read only' : v.use_tabs ? 'Tabs' : `${v.tab} spaces`
    $('st-eol').textContent = v.crlf ? 'CRLF' : 'LF'
    if (!state.running) $('st-msg').textContent = v.msg ?? ''
    const plural = (n, s) => `${n} ${s}${n === 1 ? '' : 's'}`
    $('st-diag').textContent = v.errors || v.warnings ? `${plural(v.errors, 'error')}  ${plural(v.warnings, 'warning')}` : ''
    if (v.all_bps) {
      state.bps.set(tab.path, new Set(v.all_bps))
      saveSession()
      app.debug?.syncBps(tab.path, v.all_bps)
    }
    if (!tab.virtual) {
      const c = cursors[tab.path]
      if (!c || c[0] !== v.line || c[1] !== v.col - 1) {
        cursors[tab.path] = [v.line, v.col - 1]
        saveSession()
      }
    }
    if (lastRev.get(id) !== v.rev) {
      const seen = lastRev.has(id)
      lastRev.set(id, v.rev)
      if (seen && !tab.virtual) lastEdit = { path: tab.path, line: v.line, col: v.col - 1 }
      if (seen) {
        clearTimeout(lspTimer)
        lspTimer = setTimeout(() => intel.syncDoc(id), 450)
      }
    }
    if (findOpen) {
      clearTimeout(findCountTimer)
      findCountTimer = setTimeout(updateCount, 200)
    }
  },
})

let lastEdit = null
let problemsTimer = 0
function requestSemantic(id) {
  if (!theme.isColored() || state.lsp !== 'on') return
  invoke('lsp_semantic', { id }).then(() => id === state.active && ed.repaint()).catch(() => {})
}

app.requestSemanticAll = () => state.tabs.filter((t) => !t.virtual).forEach((t) => requestSemantic(t.id))

listen('diag', (e) => {
  if (e.payload === state.active) ed.repaint()
  requestSemantic(e.payload)
  clearTimeout(problemsTimer)
  problemsTimer = setTimeout(refreshProblems, 300)
  if (state.left === 'structure' && e.payload === state.active) refreshStructure()
})

$('st-diag').addEventListener('click', () => showBottom('problems'))

let structureToken = 0

async function refreshStructure() {
  if (state.left !== 'structure') return
  const el = $('v-structure')
  const id = state.active
  if (id === null || state.lsp !== 'on') {
    el.replaceChildren(h('div', { class: 'notice' }, state.lsp === 'on' ? 'No file open' : 'clangd is off'))
    return
  }
  const token = ++structureToken
  try {
    const syms = await invoke('lsp_symbols', { id })
    if (token !== structureToken) return
    if (syms.length) intel.renderSymbols(el, syms)
    else el.replaceChildren(h('div', { class: 'notice' }, 'Nothing to show'))
  } catch (e) {
    el.replaceChildren(h('div', { class: 'notice' }, String(e)))
  }
}
app.refreshStructure = refreshStructure

const rel = (p) => (state.root && p.startsWith(state.root + SEP) ? p.slice(state.root.length + 1) : p)

function groupHits(el, groups, renderHit) {
  el.append(
    ...[...groups].flatMap(([path, items]) => [
      h('div', { class: 'group' }, basename(path), h('small', {}, rel(dirname(path)))),
      ...items.map(renderHit),
    ]),
  )
}

async function refreshProblems() {
  if (state.bottom !== 'problems') return
  const el = $('v-problems')
  const groups = new Map()
  for (const t of state.tabs) {
    if (t.virtual) continue
    const d = await invoke('doc_diags', { id: t.id })
    if (d.length) groups.set(t.path, d)
  }
  for (const i of build.buildIssues) {
    if (!groups.has(i.path)) groups.set(i.path, [])
    if (!groups.get(i.path).some((x) => x.line === i.line && x.msg === i.msg)) groups.get(i.path).push(i)
  }
  el.replaceChildren()
  if (!groups.size) {
    el.append(h('div', { class: 'notice' }, 'No problems in open files'))
    return
  }
  const paths = new Map([...groups].map(([p, items]) => [p, items.map((i) => ({ ...i, path: p }))]))
  groupHits(el, paths, (i) => h('div', { class: 'hit', onclick: () => guard(jumpTo(i.path, i.line, i.col)) }, h('span', { class: 'sev' + (i.sev === 1 ? ' e' : '') }, i.sev === 1 ? 'E' : i.sev === 2 ? 'W' : 'I'), h('span', { class: 'ln' }, i.line + 1), i.msg))
}
app.refreshProblems = () => guard(refreshProblems())

function showHits(title, sub, hits, header = null) {
  showBottom('find')
  const el = $('v-find')
  el.replaceChildren(...(header ? [header] : []))
  $('panel-title').textContent = title
  $('panel-sub').textContent = sub
  const groups = new Map()
  for (const x of hits) {
    if (!groups.has(x.path)) groups.set(x.path, [])
    groups.get(x.path).push(x)
  }
  groupHits(el, groups, (x) => h('div', { class: 'hit', onclick: () => guard(jumpTo(x.path, x.line, x.col)) }, h('span', { class: 'ln' }, x.line + 1), h('code', {}, x.text)))
}
app.showHits = showHits

async function projectSearch() {
  if (!state.root) return
  await saveAll(true)
  const input = h('input', { placeholder: 'Find in project, Enter to search', spellcheck: 'false', value: lastGrep })
  const header = h('div', { class: 'search-row' }, input)
  const go = async () => {
    lastGrep = input.value
    if (lastGrep.length < 2) return
    const hits = await invoke('grep', { root: state.root, query: lastGrep })
    showHits('Find in Project', `${hits.length}${hits.length >= 400 ? '+' : ''} found`, hits, header)
    input.focus()
  }
  input.addEventListener('keydown', (e) => e.key === 'Enter' && guard(go()))
  showHits('Find in Project', '', [], header)
  input.focus()
  input.select()
}
let lastGrep = ''

let findOpen = false
let findCase = false
let findCountTimer = 0
let findTimer = 0
const fq = $('find-q'), fr = $('find-r'), fn = $('find-n')

function openFind() {
  if (state.active === null) return
  findOpen = true
  $('findbar').hidden = false
  fq.focus()
  fq.select()
  runSearch()
}

function closeFind() {
  findOpen = false
  $('findbar').hidden = true
  if (state.active !== null) {
    invoke('doc_search', { id: state.active, q: '', case: false }).then(() => ed.repaint())
    ed.focus()
  }
}

async function updateCount() {
  if (!findOpen || state.active === null) return
  const n = await invoke('doc_search', { id: state.active, q: fq.value, case: findCase })
  fn.textContent = fq.value ? `${n} found` : ''
}

async function runSearch() {
  await updateCount()
  ed.repaint()
}

fq.addEventListener('input', () => {
  clearTimeout(findTimer)
  findTimer = setTimeout(async () => {
    await runSearch()
    if (fq.value) ed.send({ t: 'find_next', back: false })
  }, 120)
})
fq.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') ed.send({ t: 'find_next', back: e.shiftKey })
})
fr.addEventListener('keydown', (e) => {
  if (e.key === 'Enter') ed.send({ t: 'replace_one', s: fr.value })
})
$('find-next').addEventListener('click', () => ed.send({ t: 'find_next', back: false }))
$('find-prev').addEventListener('click', () => ed.send({ t: 'find_next', back: true }))
$('find-one').addEventListener('click', () => ed.send({ t: 'replace_one', s: fr.value }))
$('find-all').addEventListener('click', () => ed.send({ t: 'replace_all', s: fr.value }))
$('find-x').addEventListener('click', closeFind)
$('find-case').addEventListener('click', (e) => {
  findCase = !findCase
  e.currentTarget.setAttribute('aria-pressed', String(findCase))
  runSearch()
})

let recent = []

async function quickOpen() {
  if (!state.root) return
  if (!state.files) state.files = await invoke('list_files', { root: state.root })
  palette({
    placeholder: 'Open file by name',
    query: async (q) => {
      const hits = q ? state.files.map((f) => [fuzzy(q, f), f]).filter((x) => x[0] >= 0).sort((a, b) => b[0] - a[0]).slice(0, 40).map((x) => x[1]) : state.files.slice(0, 40)
      return hits.map((f) => ({ f, open: () => guard(jumpToFile(join(state.root, f))) }))
    },
    render: ({ f }) => h('div', {}, h('b', {}, basename(f)), h('span', {}, f.includes(SEP) ? dirname(f) : '')),
  })
}

async function jumpToFile(path) {
  navPush()
  await openFile(path)
}

function recentFiles() {
  palette({
    placeholder: 'Recent files',
    query: async (q) => recent.filter((p) => p !== activeTab()?.path && (!q || fuzzy(q, p) >= 0)).map((p) => ({ p, open: () => guard(jumpToFile(p)) })),
    render: ({ p }) => h('div', {}, h('b', {}, basename(p)), h('span', {}, rel(dirname(p)))),
  })
}

function findAction() {
  palette({
    placeholder: 'Type a command',
    query: async (q) =>
      [...actions.values()]
        .filter((a) => !a.hidden)
        .map((a) => [q ? fuzzy(q, a.title) : 0, a])
        .filter((x) => x[0] >= 0)
        .sort((a, b) => b[0] - a[0])
        .slice(0, 40)
        .map(([, a]) => ({ a, open: () => runAction(a.id) })),
    render: ({ a }) => h('div', {}, h('b', {}, a.title), a.key ? h('kbd', {}, keyLabel(a.key)) : null),
  })
}

async function gotoLine() {
  if (state.active === null) return
  const v = await promptBox('Go to line', '', 'Go')
  const n = parseInt(v, 10)
  if (n > 0) {
    navPush()
    ed.send({ t: 'goto', line: n - 1, col: 0 })
  }
}

const openSettings = (section) => settingsPage.open(typeof section === 'string' ? section : undefined)
app.openSettings = openSettings
async function toggleChat(on = state.right !== 'chat') {
  if (on && !settings.aiChat) return toast('Turn on AI chat in Settings > AI')
  state.right = on ? 'chat' : null
  $('right').hidden = $('right-rz').hidden = !on
  $('rail-chat').classList.toggle('on', on)
  $('chat-toggle').classList.toggle('on', on)
  store('chatOpen', on)
  if (!on) return ed.focus()
  const c = await loadChat()
  $('right-actions').replaceChildren(c.panelActions())
  c.shown()
}
app.toggleChat = toggleChat
app.syncChat = () => {
  $('rail-chat').hidden = !settings.aiChat
  if (!settings.aiChat && state.right) toggleChat(false)
}
/** The button and Alt+8: turn the chat on if it is off, then open or close it. */
function chatButton() {
  if (!settings.aiChat) {
    settings.aiChat = true
    store('aiChat', true)
    closeNote('ai')
    app.syncChat()
  }
  return toggleChat(state.right !== 'chat')
}
$('chat-toggle').addEventListener('click', chatButton)
$('rail-chat').addEventListener('click', () => toggleChat())
$('right-x').addEventListener('click', () => toggleChat(false))

function quickTheme() {
  palette({
    placeholder: 'Switch theme',
    query: async (q) =>
      theme
        .themes()
        .filter((t) => !q || fuzzy(q, t.name) >= 0)
        .map((t) => ({
          t,
          open: () => {
            theme.setTheme(t.id)
            applyLook()
            app.requestSemanticAll()
          },
        })),
    render: ({ t }) => h('div', {}, h('b', {}, t.name), t.id === theme.look.themeId ? h('span', { class: 'kind' }, 'current') : null),
  })
}

$('quick').addEventListener('click', () => (state.root ? guard(quickOpen()) : findAction()))

const cmd = (c) => () => ed.send(c)
const editorOnly = { editor: true }
const activeDo = (fn) => () => {
  const t = activeTab()
  if (t) return fn(t)
}

reg('open-folder', 'Open Folder…', null, pickFolder)
reg('new-project', 'New Project…', null, newProject)
reg('new-file', 'New File…', 'A-Insert', () => state.root && createEntry(activeTab() && !activeTab().virtual ? dirname(activeTab().path) : state.root, false))
reg('save', 'Save', 'C-s', activeDo((t) => saveTab(t)))
reg('save-all', 'Save All', 'C-S-s', () => saveAll())
reg('close-tab', 'Close Tab', 'C-F4', activeDo(closeTab))
reg('close-project', 'Close Project', null, async () => {
  await closeProject()
  state.root = null
  $('work').hidden = true
  $('status').hidden = true
  $('run-tools').hidden = true
  $('home').hidden = false
  $('proj-chip').hidden = true
  $('st-git').textContent = ''
  renderHome()
  renderMenus()
})
reg('settings', 'Settings…', 'C-A-s', openSettings)
reg('fullscreen', 'Toggle Full Screen', 'F11', async () => win.setFullscreen(!(await win.isFullscreen())))
reg('quit', 'Exit', null, requestClose)

reg('undo', 'Undo', 'C-z', cmd({ t: 'undo' }), editorOnly)
reg('redo', 'Redo', 'C-S-z', cmd({ t: 'redo' }), editorOnly)
reg('select-all', 'Select All', 'C-a', cmd({ t: 'select_all' }), editorOnly)
reg('expand', 'Extend Selection', 'C-w', cmd({ t: 'expand_selection' }), editorOnly)
reg('shrink', 'Shrink Selection', 'C-S-w', cmd({ t: 'shrink_selection' }), editorOnly)
reg('comment', 'Toggle Comment', 'C-/', cmd({ t: 'toggle_comment' }), editorOnly)
reg('comment-it', 'Toggle Comment', 'C-S-/', cmd({ t: 'toggle_comment' }), { ...editorOnly, hidden: true })
reg('duplicate', 'Duplicate Line', 'C-d', cmd({ t: 'duplicate_line' }), editorOnly)
reg('delete-line', 'Delete Line', 'C-y', cmd({ t: 'delete_line' }), editorOnly)
reg('move-up', 'Move Line Up', 'A-S-ArrowUp', cmd({ t: 'move_lines', up: true }), editorOnly)
reg('move-down', 'Move Line Down', 'A-S-ArrowDown', cmd({ t: 'move_lines', up: false }), editorOnly)
reg('join', 'Join Lines', 'C-S-j', cmd({ t: 'join_lines' }), editorOnly)
reg('line-below', 'Start New Line', 'S-Enter', cmd({ t: 'new_line_below' }), editorOnly)
reg('line-above', 'Start New Line Above', 'C-A-Enter', cmd({ t: 'new_line_above' }), editorOnly)
reg('complete-statement', 'Complete Current Statement', 'C-S-Enter', cmd({ t: 'complete_statement' }), editorOnly)
reg('toggle-case', 'Toggle Case', 'C-S-u', cmd({ t: 'toggle_case' }), editorOnly)
reg('goto-bracket', 'Move Caret to Matching Bracket', 'C-S-m', cmd({ t: 'goto_bracket' }), editorOnly)
reg('last-edit', 'Last Edit Location', 'C-S-Backspace', () => lastEdit && guard(jumpTo(lastEdit.path, lastEdit.line, lastEdit.col)), editorOnly)
reg('fold', 'Fold Block', 'C--', cmd({ t: 'fold_caret', fold: true }), editorOnly)
reg('unfold', 'Unfold Block', 'C-=', cmd({ t: 'fold_caret', fold: false }), editorOnly)
reg('unfold-it', 'Unfold Block', 'C-S-+', cmd({ t: 'fold_caret', fold: false }), { ...editorOnly, hidden: true })
reg('fold-all', 'Fold All', 'C-S--', cmd({ t: 'fold_all', fold: true }), editorOnly)
reg('unfold-all', 'Unfold All', 'C-S-=', cmd({ t: 'fold_all', fold: false }), editorOnly)
reg('find', 'Find', 'C-f', openFind)
reg('replace', 'Replace', 'C-r', openFind)
reg('find-project', 'Find in Project', 'C-S-f', projectSearch)

reg('quick-open', 'Go to File…', 'C-p', quickOpen)
reg('quick-open-2', 'Go to File…', 'C-S-n', quickOpen, { hidden: true })
reg('recent', 'Recent Files…', 'C-e', recentFiles)
reg('actions', 'Find Action…', 'C-S-a', findAction)
reg('goto-line', 'Go to Line…', 'C-g', gotoLine)
reg('back', 'Back', 'C-A-ArrowLeft', () => navGo(nav.back, nav.forward))
reg('forward', 'Forward', 'C-A-ArrowRight', () => navGo(nav.forward, nav.back))
reg('next-problem', 'Next Problem', 'F2', async () => {
  if (state.active === null) return
  await ed.idle()
  await invoke('doc_next_problem', { id: state.active })
  ed.send()
})
reg('next-tab', 'Next Tab', 'C-Tab', () => cycleTab(1))
reg('prev-tab', 'Previous Tab', 'C-S-Tab', () => cycleTab(-1))

reg('debug', 'Debug', 'S-F9', async () => (await loadDebug()).start())
reg('toggle-bp', 'Toggle Breakpoint', 'C-F8', () => ed.quiet({ t: 'toggle_breakpoint', row: null }), editorOnly)

reg('tw-project', 'Project', 'A-1', () => showLeft('project'))
reg('tw-structure', 'Structure', 'A-7', () => showLeft('structure'))
reg('tw-git', 'Commit', 'A-0', () => showLeft('git'))
reg('tw-find', 'Find', 'A-3', () => showBottom('find', false))
reg('tw-build', 'Build', 'A-b', () => showBottom('build', false))
reg('tw-run', 'Run', 'A-4', () => showBottom('run', false))
reg('tw-debug', 'Debug', 'A-5', () => showBottom('debug', false))
reg('tw-problems', 'Problems', 'A-6', () => showBottom('problems', false))
reg('tw-terminal', 'Terminal', 'A-F12', () => showBottom('terminal', false))
reg('tw-chat', 'AI Chat', 'A-8', () => chatButton())
reg('switch-theme', 'Switch Theme…', 'C-`', quickTheme)

function keymap() {
  let close
  const rows = [...actions.values()].filter((a) => a.key && !a.hidden)
  close = modal(h('div', { class: 'dialog', style: 'width:min(560px,calc(100vw - 32px))' }, h('h3', {}, 'Keyboard shortcuts'), h('div', { class: 'keys', style: 'max-height:60vh;overflow:auto;padding-right:6px' }, ...rows.flatMap((a) => [h('span', {}, a.title), h('kbd', {}, keyLabel(a.key))])), h('div', { class: 'actions' }, h('button', { class: 'primary', onclick: () => close() }, 'Done'))))
}
reg('keymap', 'Keyboard Shortcuts', null, keymap)
reg('install-cli', 'Install clight and cl Commands', null, async () => {
  const names = await invoke('install_cli')
  if (SEP === '\\') toast(`Installed ${names}. Open a new terminal to use them`)
  else toast(names ? `Installed ${names} in ~/.local/bin` : 'Those command names are already taken')
})

$('debug').addEventListener('click', () => runAction('debug'))
$('rail-gear').addEventListener('click', () => runAction('settings'))

function cycleTab(d) {
  if (state.tabs.length < 2) return
  const i = state.tabs.findIndex((t) => t.id === state.active)
  activate(state.tabs[(i + d + state.tabs.length) % state.tabs.length].id)
}

const MENUS = {
  File: ['open-folder', 'new-project', 'new-file', '-', 'save', 'save-all', 'close-tab', '-', 'settings', 'close-project', 'quit'],
  Edit: ['undo', 'redo', '-', 'find', 'replace', 'find-project', '-', 'expand', 'shrink', 'duplicate', 'delete-line', 'move-up', 'move-down', 'join', 'comment', '-', 'select-all'],
  View: ['tw-project', 'tw-structure', 'tw-git', '-', 'tw-build', 'tw-run', 'tw-debug', 'tw-problems', 'tw-find', 'tw-terminal', 'tw-chat', '-', 'fold', 'unfold', 'fold-all', 'unfold-all', '-', 'switch-theme', 'fullscreen'],
  Navigate: ['actions', 'quick-open', 'recent', 'symbols', 'file-symbols', 'goto-line', '-', 'goto-def', 'switch-header', 'usages', 'next-problem', '-', 'back', 'forward', 'next-tab', 'prev-tab'],
  Code: ['code-actions', 'quick-doc', 'rename', 'format'],
  Build: ['build', 'clean', 'reload-cmake', 'ctest', '-', 'run-config'],
  Run: ['run', 'debug', 'stop', '-', 'dbg-resume', 'dbg-over', 'dbg-into', 'dbg-out', 'dbg-cursor', '-', 'toggle-bp', 'dbg-watch'],
  Git: ['commit', 'git-pull', 'git-push', 'git-branch'],
  Help: ['keymap', 'actions', '-', ...(SEP === '\\' ? ['download-tools'] : []), 'install-cli', 'check-updates', 'about'],
}

function renderMenus() {
  const nav = $('menus')
  nav.replaceChildren()
  if (!state.root) return
  let openBtn = null
  const close = () => {
    openBtn?.classList.remove('open')
    openBtn = null
  }
  const openMenu = async (name, btn) => {
    if (name === 'Run') await loadDebug()
    const items = MENUS[name].map((id) => {
      if (id === '-') return '-'
      const a = actions.get(id)
      return [a.title, () => guard(a.run()), keyLabel(a.key)]
    })
    openBtn?.classList.remove('open')
    const r = btn.getBoundingClientRect()
    showMenu(r.left, r.bottom + 3, items, () => {
      close()
      setTimeout(() => {
        if (!menuVisible()) {
          $('bar').classList.remove('menus-open')
          fitBar()
        }
      }, 60)
    })
    btn.classList.add('open')
    openBtn = btn
  }
  $('burger').onclick = () => {
    $('bar').classList.toggle('menus-open')
    fitBar()
  }
  for (const name of Object.keys(MENUS)) {
    const b = h('button', {
      onpointerdown: (e) => {
        e.preventDefault()
        if (openBtn === b) hideMenu()
        else openMenu(name, b)
      },
      onpointerenter: () => {
        if (openBtn && openBtn !== b) openMenu(name, b)
      },
    }, name)
    nav.append(b)
  }
  fitBar()
}

function fitBar() {
  const open = !!state.root && $('bar').classList.contains('menus-open')
  $('burger').hidden = !state.root
  $('menus').hidden = !open
}
app.fitBar = fitBar

ed.hooks({
  beforeKey: intel.beforeKey,
  afterType: intel.afterType,
  typed: intel.typed,
  ctrlClick: () => app.gotoDefinition(),
  pointer: intel.hidePopups,
  scrolled: () => {
    $('hover').hidden = true
  },
  hover: intel.mouseHover,
  hoverEnd: intel.hoverEnd,
  context: (c, e) => {
    const tab = activeTab()
    const editable = !tab?.virtual
    const intelOn = state.lsp === 'on' && editable
    const item = (id, enabled = true) => {
      const a = actions.get(id)
      return [a.title, () => guard(a.run()), keyLabel(a.key), enabled]
    }
    const paused = !!state.debugging
    showMenu(e.clientX, e.clientY, [
      item('goto-def', intelOn),
      item('usages', intelOn),
      item('rename', intelOn),
      item('code-actions', intelOn),
      item('switch-header', intelOn),
      '-',
      item('format', editable),
      item('comment', editable),
      item('duplicate', editable),
      '-',
      ['Cut', () => guard(ed.cutToClipboard()), 'Ctrl+X', editable],
      ['Copy', () => guard(ed.copyToClipboard()), 'Ctrl+C'],
      ['Paste', () => guard(ed.pasteFromClipboard()), 'Ctrl+V', editable],
      item('select-all'),
      '-',
      item('toggle-bp', editable),
      ...(paused && actions.has('dbg-cursor') ? [item('dbg-cursor')] : []),
    ])
  },
  gutterContext: async (c, e) => {
    const t = activeTab()
    const v = ed.lastView()
    if (!t || !v || t.virtual) return
    const line = v.nums[c.line - v.first]
    if (line === undefined) return
    showMenu(e.clientX, e.clientY, [
      ['Toggle Breakpoint', () => ed.quiet({ t: 'toggle_breakpoint', row: c.line })],
      ['Condition…', async () => (await loadDebug()).setCondition(t.path, line), '', !!state.bps.get(t.path)?.has(line)],
    ])
  },
})

const byKey = new Map()
const keyMap = () => {
  if (byKey.size !== actions.size) {
    byKey.clear()
    for (const a of actions.values()) if (a.key) byKey.set(a.key, a)
  }
  return byKey
}


document.addEventListener('keydown', async (e) => {
  if (e.key === 'Escape') {
    if (menuVisible()) {
      hideMenu()
      e.preventDefault()
    } else if (modalOpen()) {
      closeModal()
      e.preventDefault()
    } else if (findOpen) closeFind()
    return
  }
  if (modalOpen()) return
  const name = keyName(e)
  if ((termMod?.hasFocus() || build.runTermFocused()) && !TERMINAL_PASS.has(name)) return
  if (['S-F9', 'F7', 'F8', 'F9', 'S-F8', 'A-F9'].includes(name)) await loadDebug()
  const a = keyMap().get(name)
  if (!a) return
  if (a.editor && !ed.hasFocus()) return
  e.preventDefault()
  guard(a.run())
})

document.addEventListener('contextmenu', (e) => {
  if (!e.target.closest('input, .out, #hover, textarea, .chat-log, .tgrid')) e.preventDefault()
})

$('w-min').addEventListener('click', () => win.minimize())
$('w-max').addEventListener('click', () => win.toggleMaximize())
$('w-close').addEventListener('click', requestClose)
for (const el of document.querySelectorAll('[data-rz]')) {
  el.addEventListener('pointerdown', (e) => {
    e.preventDefault()
    win.startResizeDragging(el.dataset.rz)
  })
}

let maxTimer = 0
addEventListener('resize', () => {
  clearTimeout(maxTimer)
  maxTimer = setTimeout(async () => {
    const max = await win.isMaximized()
    if (max) root.dataset.maximized = ''
    else delete root.dataset.maximized
    $('w-max-use').setAttribute('href', max ? '#i-restore' : '#i-max')
    if (!(await win.isFullscreen())) store('window', max ? { ...load('window', {}), max: true } : { w: innerWidth, h: innerHeight, max: false })
  }, 150)
})

async function requestClose() {
  await ed.idle()
  await syncDirty()
  const dirty = state.tabs.filter((t) => t.dirty)
  if (dirty.length) {
    const save = await confirmBox(`${dirty.length} unsaved file${dirty.length > 1 ? 's' : ''}`, dirty.map((t) => t.name).join(', '), 'Save all', 'Quit without saving')
    if (save && !(await saveAll())) return
  }
  if (state.root) {
    clearTimeout(sessionTimer)
    store('session:' + state.root, sessionSnapshot())
  }
  invoke('app_quit')
}

listen('close-requested', requestClose)

addEventListener('focus', async () => {
  root.dataset.focus = ''
  if (!state.root) return
  await app.afterExternalChange()
  git.refresh()
})
addEventListener('blur', () => {
  delete root.dataset.focus
  if (state.root && settings.autosave) guard(saveAll(true))
})

function drag(el, varName, min, max, getBase) {
  el.addEventListener('pointerdown', (e) => {
    e.preventDefault()
    el.setPointerCapture(e.pointerId)
    let raf = 0
    const move = (ev) => {
      if (raf) return
      raf = requestAnimationFrame(() => {
        raf = 0
        root.style.setProperty(varName, Math.max(min, Math.min(max(), getBase(ev))) + 'px')
      })
    }
    const up = () => {
      el.removeEventListener('pointermove', move)
      el.removeEventListener('pointerup', up)
      store('layout', { side: root.style.getPropertyValue('--side'), panel: root.style.getPropertyValue('--panel'), right: root.style.getPropertyValue('--right') })
    }
    el.addEventListener('pointermove', move)
    el.addEventListener('pointerup', up)
  })
}
drag($('side-rz'), '--side', 140, () => innerWidth * 0.5, (e) => e.clientX)
drag($('right-rz'), '--right', 280, () => innerWidth * 0.6, (e) => innerWidth - e.clientX - 5)
drag($('panel-rz'), '--panel', 90, () => innerHeight - 180, (e) => innerHeight - 22 - e.clientY)

if (SEP === '\\') for (const el of document.querySelectorAll('#debug, [data-bottom="debug"]')) el.hidden = true

async function start() {
  root.dataset.focus = ''
  const layout = load('layout', null)
  if (layout?.side) root.style.setProperty('--side', layout.side)
  if (layout?.panel) root.style.setProperty('--panel', layout.panel)
  if (layout?.right) root.style.setProperty('--right', layout.right)
  await document.fonts.load('13px "JetBrains Mono Variable"').catch(() => {})
  state.sys = await invoke('sys_info')
  applyLook()
  const w = load('window', null)
  if (w?.max) await win.maximize().catch(() => {})
  else if (w?.w >= 640 && w?.h >= 400) await win.setSize(new window.__TAURI__.window.LogicalSize(w.w, w.h)).catch(() => {})
  renderHome()
  intel.setLsp('off')
  const arg = await invoke('initial_path')
  if (arg) {
    await guard(openProject(arg.root))
    if (arg.file) await guard(openFile(arg.file))
  }
  await invoke('app_ready')
  update.startup()
  guard(tools.startup())
  app.syncChat()
  if (settings.aiChat && load('chatOpen', false)) toggleChat(true)
  ai.startup()
}

start()
