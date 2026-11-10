import * as ed from './editor.js'
import { $, h, icon, iconBtn, invoke, listen, app, state, reg, toast, guard, load, store, promptBox } from './core.js'
import { buildForRun, parsedArgs, parsedEnv, syncButtons, buildOut } from './build.js'

const q = (s) => JSON.stringify(s)
const view = $('v-debug')
let session = false
let paused = false
let frames = []
let frame = 0
let thread = '1'
let varobjs = []
let gdbBps = new Map()
let execDoc = null
let watches = []
let consoleWanted = false

const cmd = (c) => invoke('dbg_cmd', { cmd: c })

const framesEl = h('div', {})
const varsEl = h('div', {})
const watchEl = h('div', {})
const consoleEl = h('div', { class: 'out', id: 'dbg-console', tabindex: '0' })
const inputEl = h('input', { id: 'dbg-input', spellcheck: 'false', placeholder: 'Program input' })

view.replaceChildren(
  h(
    'div',
    { class: 'dbg' },
    h('div', {}, h('h4', {}, 'Frames'), framesEl),
    h('div', {}, h('h4', {}, 'Variables'), varsEl, h('h4', { style: 'display:flex;align-items:center' }, 'Watches', h('span', { class: 'grow' }), iconBtn('plus', 'Add watch', addWatch)), watchEl),
    h('div', {}, consoleEl, inputEl),
  ),
)

function log(text, cls) {
  const follow = consoleEl.scrollTop + consoleEl.clientHeight >= consoleEl.scrollHeight - 30
  const last = consoleEl.lastChild
  const clean = text.replace(/\r/g, '')
  if (last && last.dataset.open && last.className === (cls ?? '')) {
    last.textContent += clean
    if (clean.endsWith('\n')) delete last.dataset.open
  } else {
    const el = h('div', { class: cls ?? '' }, clean)
    if (!clean.endsWith('\n')) el.dataset.open = '1'
    consoleEl.append(el)
  }
  while (consoleEl.childElementCount > 1500) consoleEl.firstChild.remove()
  if (follow) consoleEl.scrollTop = consoleEl.scrollHeight
}

listen('dbg-out', (e) => log(e.payload))

listen('dbg', (e) => {
  const m = e.payload
  if (m.kind === 'console') {
    if (session && consoleWanted) log(m.text, 'hd')
    return
  }
  if (m.kind === 'exit') return finish()
  if (m.kind === 'exec' && m.class === 'running') return onRunning()
  if (m.kind === 'exec' && m.class === 'stopped') guard(onStopped(m.data))
})

function onRunning() {
  paused = false
  clearExec()
  syncTools()
  inputEl.placeholder = 'Program input'
}

function clearExec() {
  if (execDoc !== null) {
    ed.docCommand(execDoc, [{ t: 'set_exec', line: null }])
    execDoc = null
  }
}

async function onStopped(d) {
  if (d.reason?.startsWith('exited')) {
    log(`Process exited${d['exit-code'] ? ' with code ' + parseInt(d['exit-code'], 8) : ''}\n`, 'hd')
    await stop()
    return
  }
  paused = true
  thread = d['thread-id'] ?? thread
  syncTools()
  inputEl.placeholder = 'gdb command'
  if (d.reason === 'signal-received') {
    log(`Stopped by ${d['signal-name']} (${d['signal-meaning']})\n`, 'er')
    toast(`Program received ${d['signal-name']}`)
  }
  const r = await cmd(`-stack-list-frames --thread ${thread} 0 63`)
  frames = r.stack ?? []
  const i = frames.findIndex((f) => f.fullname)
  await selectFrame(Math.max(0, i))
  app.showBottom('debug')
  ed.focus()
}

async function selectFrame(i) {
  frame = i
  renderFrames()
  const f = frames[i]
  await cmd(`-stack-select-frame --thread ${thread} ${i}`).catch(() => {})
  await dropVarobjs()
  if (f?.fullname && (await invoke('path_exists', { path: f.fullname }))) {
    clearExec()
    await app.jumpTo(f.fullname, f.line - 1, 0)
    execDoc = state.active
    ed.quiet({ t: 'set_exec', line: f.line - 1 })
  }
  await refreshVars()
}

function renderFrames() {
  framesEl.replaceChildren(
    ...frames.map((f, i) =>
      h(
        'div',
        { class: 'fr' + (i === frame ? ' on' : ''), onclick: () => guard(selectFrame(i)) },
        f.func ?? '??',
        h('small', {}, f.file ? `${f.file.split('/').pop()}:${f.line}` : (f.from ?? '').split('/').pop()),
      ),
    ),
  )
}

async function dropVarobjs() {
  const old = varobjs
  varobjs = []
  await Promise.all(old.map((v) => cmd(`-var-delete ${q(v)}`).catch(() => {})))
}

function varRow(name, value, type, depth, expand) {
  const row = h('div', { class: 'var' }, h('span', { class: 'tw' }, expand ? '›' : ''), h('span', { class: 'nm' }, name), h('span', { class: 'vl' }, value ?? ''), h('span', { class: 'ty' }, type ?? ''))
  row.style.paddingLeft = 4 + depth * 12 + 'px'
  if (expand) {
    let kids = null
    row.addEventListener('click', async () => {
      if (kids) {
        kids.forEach((k) => k.remove())
        kids = null
        row.classList.remove('open')
        return
      }
      try {
        row.classList.add('open')
        kids = await expand()
        row.after(...kids)
      } catch (e) {
        row.classList.remove('open')
        toast(e)
      }
    })
  }
  return row
}

const childrenOf = (obj, depth) => async () => {
  const r = await cmd(`-var-list-children --all-values ${q(obj)} 0 200`)
  const rows = []
  for (const c of r.children ?? []) {
    if (!c.type && ['public', 'private', 'protected'].includes(c.exp)) {
      rows.push(...(await childrenOf(c.name, depth)()))
      continue
    }
    const more = Number(c.numchild) > 0 || c.dynamic === '1'
    rows.push(varRow(c.exp, c.value, c.type, depth, more ? childrenOf(c.name, depth + 1) : null))
  }
  return rows
}

async function refreshVars() {
  if (!paused) return
  const r = await cmd(`-stack-list-variables --thread ${thread} --frame ${frame} --simple-values`).catch(() => ({ variables: [] }))
  const rows = []
  for (const v of (r.variables ?? []).slice(0, 64)) {
    if (v.value !== undefined) {
      rows.push(varRow(v.name, v.value, v.type, 0, null))
      continue
    }
    try {
      const o = await cmd(`-var-create - * ${q(v.name)}`)
      varobjs.push(o.name)
      const more = Number(o.numchild) > 0 || o.dynamic === '1'
      rows.push(varRow(v.name, o.value, v.type, 0, more ? childrenOf(o.name, 1) : null))
    } catch {
      rows.push(varRow(v.name, '', v.type, 0, null))
    }
  }
  varsEl.replaceChildren(...rows)
  if (!rows.length) varsEl.replaceChildren(h('div', { class: 'notice' }, 'No variables in this frame'))
  await refreshWatches()
}

function saveWatches() {
  store('watches:' + state.root, watches)
}

async function refreshWatches() {
  const rows = await Promise.all(
    watches.map(async (w, i) => {
      let value = ''
      if (paused) value = await cmd(`-data-evaluate-expression ${q(w)}`).then((r) => r.value, (e) => e)
      const row = varRow(w, value, null, 0, null)
      row.append(h('span', { class: 'rm', onclick: (ev) => (ev.stopPropagation(), watches.splice(i, 1), saveWatches(), guard(refreshWatches())) }, '×'))
      return row
    }),
  )
  watchEl.replaceChildren(...rows)
}

async function addWatch() {
  const w = await promptBox('Add watch', '', 'Add')
  if (!w) return
  watches.push(w)
  saveWatches()
  await refreshWatches()
}

const toolbar = h('span', { class: 'headacts' })
const tools = {
  resume: ['resume', 'Resume (F9)', () => step('-exec-continue')],
  pause: ['pause', 'Pause', () => cmd('-exec-interrupt')],
  over: ['over', 'Step Over (F8)', () => step('-exec-next')],
  into: ['into', 'Step Into (F7)', () => step('-exec-step')],
  out: ['out', 'Step Out (Shift+F8)', () => step('-exec-finish')],
  cursor: ['cursor', 'Run to Cursor (Alt+F9)', runToCursor],
  stop: ['stop', 'Stop', stop],
}
const toolEls = {}
for (const [k, [ic, title, fn]] of Object.entries(tools)) {
  toolEls[k] = h('button', { class: 'iconbtn ghost tiny', title, onclick: () => guard(fn()) }, icon(ic))
  toolbar.append(toolEls[k])
}

function syncTools() {
  for (const k of ['resume', 'over', 'into', 'out', 'cursor']) toolEls[k].disabled = !(session && paused)
  toolEls.pause.disabled = !(session && !paused)
  toolEls.stop.disabled = !session
  state.debugging = session
  syncButtons()
  $('panel-sub').textContent = state.bottom === 'debug' ? (session ? (paused ? 'Paused' : 'Running') : '') : ''
}

export const panelActions = () => toolbar
export const refreshHeader = syncTools

function step(c) {
  if (!session || !paused) return
  return cmd(`${c} --thread ${thread}`)
}

async function runToCursor() {
  if (!session || !paused) return
  await ed.idle()
  const t = state.tabs.find((x) => x.id === state.active)
  const v = ed.lastView()
  if (t && v) await cmd(`-exec-until ${q(`${t.path}:${v.line + 1}`)}`)
}

async function insertBp(path, line) {
  const r = await cmd(`-break-insert -f ${q(`${path}:${line + 1}`)}`)
  const number = r.bkpt?.number
  if (number) {
    gdbBps.set(`${path}:${line}`, number)
    const c = state.conds.get(`${path}:${line}`)
    if (c) await cmd(`-break-condition ${number} ${c}`).catch(toast)
  }
}

export async function syncBps(path, lines) {
  if (!session) return
  const want = new Set(lines.map((l) => `${path}:${l}`))
  for (const [k, n] of [...gdbBps]) {
    if (k.startsWith(path + ':') && !want.has(k)) {
      gdbBps.delete(k)
      await cmd(`-break-delete ${n}`).catch(() => {})
    }
  }
  for (const l of lines) if (!gdbBps.has(`${path}:${l}`)) await insertBp(path, l).catch(toast)
}

export async function setCondition(path, line) {
  const key = `${path}:${line}`
  const cur = state.conds.get(key) ?? ''
  const c = await promptBox(`Breakpoint condition at line ${line + 1}`, cur, 'Set')
  if (c === null) return
  if (c) state.conds.set(key, c)
  else state.conds.delete(key)
  app.saveSession?.()
  const n = gdbBps.get(key)
  if (n) await cmd(`-break-condition ${n} ${c}`).catch(toast)
}

export async function start() {
  const b = await buildForRun()
  if (!b) return
  if (state.profile === 'Release') toast('Release builds have no debug info. Switch to Debug for full support')
  watches = load('watches:' + state.root, [])
  consoleEl.replaceChildren()
  framesEl.replaceChildren()
  varsEl.replaceChildren()
  gdbBps.clear()
  app.showBottom('debug')
  try {
    const tty = await invoke('dbg_start', { program: b.exe, cwd: b.cwd })
    session = true
    paused = false
    syncTools()
    for (const c of ['-gdb-set mi-async on', '-gdb-set confirm off', '-gdb-set pagination off', '-gdb-set debuginfod enabled off', '-enable-pretty-printing']) await cmd(c).catch(() => {})
    await cmd(`-inferior-tty-set ${tty}`)
    await cmd(`-environment-cd ${q(b.cwd)}`)
    const args = parsedArgs()
    if (args.length) await cmd(`-exec-arguments ${args.map(q).join(' ')}`)
    for (const [k, v] of parsedEnv()) await cmd(`-gdb-set environment ${k}=${v}`).catch(() => {})
    for (const [path, set] of state.bps) for (const l of set) await insertBp(path, l).catch(() => {})
    await refreshWatches()
    log(`Debugging ${b.exe.split('/').pop()}\n`, 'hd')
    await cmd('-exec-run')
  } catch (e) {
    buildOut.append(`${e}\n`, 'er')
    toast(e)
    await stop()
  }
}

export async function stop() {
  if (!session) return
  session = false
  await invoke('dbg_stop').catch(() => {})
  finish()
}

function finish() {
  session = false
  paused = false
  frames = []
  varobjs = []
  gdbBps.clear()
  clearExec()
  framesEl.replaceChildren()
  varsEl.replaceChildren()
  syncTools()
  inputEl.placeholder = 'Program input'
}

export async function evaluateHover(word) {
  if (!session || !paused) return null
  try {
    return (await cmd(`-data-evaluate-expression ${q(word)}`)).value
  } catch {
    return null
  }
}

inputEl.addEventListener('keydown', async (e) => {
  if (e.key !== 'Enter' || !session) return
  const text = inputEl.value
  inputEl.value = ''
  if (paused) {
    log(`(gdb) ${text}\n`, 'hd')
    consoleWanted = true
    await cmd(`-interpreter-exec console ${q(text)}`).catch((err) => log(`${err}\n`, 'er'))
    consoleWanted = false
    await refreshVars().catch(() => {})
  } else invoke('dbg_input', { text: text + '\n' }).catch(toast)
})

syncTools()

reg('dbg-resume', 'Resume Program', 'F9', () => step('-exec-continue'))
reg('dbg-over', 'Step Over', 'F8', () => step('-exec-next'))
reg('dbg-into', 'Step Into', 'F7', () => step('-exec-step'))
reg('dbg-out', 'Step Out', 'S-F8', () => step('-exec-finish'))
reg('dbg-cursor', 'Run to Cursor', 'A-F9', runToCursor)
reg('dbg-watch', 'Add Watch…', null, addWatch)
