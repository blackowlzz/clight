import * as ed from './editor.js'
import { $, h, invoke, listen, app, state, store, load, reg, toast, guard, modal, join, activeTab, showMenu } from './core.js'

const MAX_OUT = 3000
const DIAG_RE = /^(.+?):(\d+):(?:(\d+):)?\s+(fatal error|error|warning|note):\s*(.*)/
const MSVC_RE = /^\s*(?:\d+>)?(.+?)\((\d+)(?:,(\d+))?(?:,\d+)*\)\s*:\s+(fatal error|error|warning|note)(?:\s+\w+)?:\s*(.*?)(?:\s+\[[^\]]*\])?$/
const FAILURE_RE = /(?:^|\s)(?:fatal )?error(?: [A-Z]+\d+)?:|undefined reference to|^CMake Error|\*\*\* .*Error \d+/
const ENGLISH_ENV = [['LC_ALL', 'C'], ['LANGUAGE', 'en'], ['VSLANG', '1033']]

class Output {
  constructor(el, stdin = null) {
    this.el = el
    this.stdin = stdin
    this.pending = ''
    this.cwd = ''
    this.tailText = document.createElement('span')
    this.tail = h('div', { class: 'tail' }, this.tailText)
    if (stdin) this.tail.append(stdin)
    el.append(this.tail)
    el.addEventListener('click', (e) => {
      if (getSelection().toString()) return
      const d = e.target.closest('[data-path]')
      if (d) guard(app.jumpTo(d.dataset.path, d.dataset.line - 1, d.dataset.col - 1))
      else if (stdin && !stdin.hidden) stdin.focus()
    })
  }

  clear() {
    this.el.replaceChildren(this.tail)
    this.tailText.textContent = ''
    this.pending = ''
  }

  line(text, cls) {
    const el = document.createElement('div')
    const m = cls ? null : DIAG_RE.exec(text) ?? MSVC_RE.exec(text)
    if (m) {
      const path = m[1].startsWith('/') || /^[A-Za-z]:/.test(m[1]) ? m[1] : join(this.cwd, m[1])
      el.className = m[4] === 'warning' ? 'wa' : m[4] === 'note' ? 'no' : 'er'
      el.dataset.path = path
      el.dataset.line = m[2]
      el.dataset.col = m[3] ?? '1'
      if (m[4] !== 'note') buildIssues.push({ path, line: m[2] - 1, col: (m[3] ?? 1) - 1, sev: m[4] === 'warning' ? 2 : 1, msg: m[5] })
    } else if (cls) el.className = cls
    else if (FAILURE_RE.test(text)) el.className = 'er'
    el.textContent = text || ' '
    return el
  }

  append(text, cls) {
    const follow = this.el.scrollTop + this.el.clientHeight >= this.el.scrollHeight - 30
    this.pending += text.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '').replace(/\r\n/g, '\n')
    const parts = this.pending.split('\n')
    this.pending = parts.pop()
    const frag = document.createDocumentFragment()
    for (const p of parts) frag.append(this.line(p, cls))
    this.el.insertBefore(frag, this.tail)
    this.tailText.textContent = this.pending
    this.tailText.className = cls ?? ''
    let extra = this.el.childElementCount - MAX_OUT
    while (extra-- > 0) this.el.firstChild.remove()
    if (follow) this.el.scrollTop = this.el.scrollHeight
  }

  flush() {
    if (this.pending) this.append('\n')
  }
}

export const buildOut = new Output($('v-build'))
export const runOut = new Output($('run-out'), $('stdin'))
export let buildIssues = []
let current = buildOut
let taskDone = null

listen('task-out', (e) => current.append(e.payload.s))
listen('task-done', (e) => {
  const resolve = taskDone
  taskDone = null
  resolve?.(e.payload)
})

function setRunning(on, interactive = false) {
  state.running = on
  syncButtons()
  $('stdin').hidden = !(on && interactive)
  if (on && interactive) $('stdin').focus()
}

export function syncButtons() {
  const busy = state.running || state.debugging
  $('run').disabled = busy
  $('build').disabled = state.running
  $('debug').disabled = busy
  $('stop').disabled = !busy
  $('stop').hidden = !busy
}

export function runTask(view, title, program, args, cwd, { background = true, interactive = false, env = ENGLISH_ENV } = {}) {
  return new Promise((resolve, reject) => {
    current = view
    view.cwd = cwd
    app.showBottom(view === runOut ? 'run' : 'build')
    view.flush()
    view.append(`${title}\n`, 'hd')
    setRunning(true, interactive)
    $('st-msg').textContent = view === runOut ? 'Running' : title
    taskDone = (code) => {
      view.flush()
      setRunning(false)
      $('st-msg').textContent = ''
      resolve(code)
    }
    invoke('task_start', { program, args, cwd, env, background }).catch((e) => {
      taskDone = null
      setRunning(false)
      view.append(`${e}\n`, 'er')
      reject(e)
    })
  })
}

const configKey = () => 'run:' + state.root
export const runConfig = () => ({ args: '', cwd: '', env: '', cmake: '', ...load(configKey(), {}) })

function splitArgs(s) {
  const out = []
  for (const m of s.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)) out.push(m[1] ?? m[2] ?? m[3])
  return out
}

export const parsedArgs = () => splitArgs(runConfig().args)

export function parsedEnv() {
  return splitArgs(runConfig().env)
    .map((p) => p.split(/=(.*)/s))
    .filter((p) => p[0])
    .map((p) => [p[0], p[1] ?? ''])
}

function editConfig() {
  const c = runConfig()
  const field = (label, value, ph) => {
    const input = h('input', { value, placeholder: ph, spellcheck: 'false' })
    return [h('label', { class: 'fieldrow' }, label, input), input]
  }
  const [a, args] = field('Program arguments', c.args, '--flag value')
  const [w, cwd] = field('Working directory', c.cwd, 'Build folder')
  const [e, env] = field('Environment variables', c.env, 'NAME=value OTHER=value')
  const [m, cmake] = field('CMake options', c.cmake, '-DOPTION=ON')
  let close
  close = modal(
    h(
      'div',
      { class: 'dialog' },
      h('h3', {}, 'Run configuration'),
      a,
      w,
      e,
      m,
      h(
        'div',
        { class: 'actions' },
        h('button', { onclick: () => close() }, 'Cancel'),
        h(
          'button',
          {
            class: 'primary',
            onclick: async () => {
              const changed = cmake.value !== c.cmake
              store(configKey(), { args: args.value, cwd: cwd.value, env: env.value, cmake: cmake.value })
              close()
              if (changed && (await hasCMake())) guard(configure())
            },
          },
          'Save',
        ),
      ),
    ),
  )
}

export function setProfile(p) {
  state.profile = p
  state.build = join(state.root, `cmake-build-${p.toLowerCase()}`)
  updateRunLabel()
}

function updateRunLabel() {
  $('runconf-label').textContent = state.target ?? (state.isCMake ? 'All targets' : 'Current file')
  $('profile-label').textContent = state.profile
}

async function changeProfile(p) {
  setProfile(p)
  await refreshTargets()
  app.saveSession?.()
  if (state.lsp === 'on') app.restartIntel?.()
}

const tick = (on) => (on ? '✓' : '')

function runMenu() {
  const r = $('runconf').getBoundingClientRect()
  const targets = state.targets.map((t) => [
    t.name,
    () => {
      state.target = t.name
      store('target:' + state.root, t.name)
      updateRunLabel()
    },
    tick(t.name === state.target),
  ])
  showMenu(r.left, r.bottom + 2, [...targets, ...(targets.length ? ['-'] : []), ['Edit Configuration…', editConfig], ['Reload CMake Project', reload]])
}

function profileMenu() {
  const r = $('profile-btn').getBoundingClientRect()
  showMenu(r.left, r.bottom + 2, ['Debug', 'Release'].map((p) => [p, () => guard(changeProfile(p)), tick(state.profile === p)]))
}

$('runconf').addEventListener('click', runMenu)
$('profile-btn').addEventListener('click', profileMenu)

export async function hasCMake() {
  return invoke('path_exists', { path: join(state.root, 'CMakeLists.txt') })
}

export async function refreshTargets() {
  state.isCMake = await hasCMake()
  state.targets = state.isCMake ? await invoke('cmake_targets', { build: state.build }) : []
  const remembered = load('target:' + state.root, null)
  state.target = state.targets.find((t) => t.name === remembered)?.name ?? state.targets[0]?.name ?? null
  updateRunLabel()
}

export async function configure() {
  const hadDb = await invoke('path_exists', { path: join(state.build, 'compile_commands.json') })
  await invoke('cmake_prepare', { build: state.build })
  const extra = splitArgs(runConfig().cmake)
  const code = await runTask(buildOut, 'cmake configure', 'cmake', ['-S', state.root, '-B', state.build, `-DCMAKE_BUILD_TYPE=${state.profile}`, '-DCMAKE_EXPORT_COMPILE_COMMANDS=ON', ...extra], state.root)
  if (code !== 0) return false
  await refreshTargets()
  if (!hadDb && state.lsp === 'on') app.restartIntel?.()
  return true
}

async function jobCount() {
  const s = await invoke('sys_info')
  const byRam = s.avail_mb ? Math.floor(s.avail_mb / 1000) : s.cores
  return Math.max(1, Math.min(s.cores, byRam))
}

async function ensureConfigured() {
  const ok = await invoke('path_exists', { path: join(state.build, 'CMakeCache.txt') })
  return ok || configure()
}

async function buildCMake(target) {
  if (!(await ensureConfigured())) return false
  const args = ['--build', state.build, '-j', String(await jobCount())]
  if (target) args.push('--target', target)
  const code = await runTask(buildOut, `build ${target ?? 'all'} [${state.profile}]`, 'cmake', args, state.root)
  buildOut.append(code === 0 ? 'Build finished\n' : `Build failed (${code})\n`, code === 0 ? 'ok' : 'er')
  if (code === 0 && !state.targets.length) await refreshTargets()
  return code === 0
}

async function buildSingle() {
  const t = activeTab()
  if (!t || !/\.(c|cc|cpp|cxx)$/.test(t.path)) {
    toast('Open a .c or .cpp file, or add a CMakeLists.txt')
    return null
  }
  const dir = join(state.root, '.clight')
  await invoke('make_dirs', { path: dir })
  const exe = join(dir, t.name.replace(/\.[^.]+$/, ''))
  const cc = t.path.endsWith('.c') ? 'gcc' : 'g++'
  const opt = state.profile === 'Release' ? '-O2' : '-g'
  const code = await runTask(buildOut, `compile ${t.name}`, cc, [opt, '-Wall', '-Wextra', t.path, '-o', exe], state.root)
  buildOut.append(code === 0 ? 'Build finished\n' : `Build failed (${code})\n`, code === 0 ? 'ok' : 'er')
  return code === 0 ? exe : null
}

function prepare() {
  buildIssues.length = 0
  buildOut.clear()
}

async function doBuild() {
  if (state.running || state.debugging || !state.root || !(await app.saveAll())) return
  prepare()
  try {
    if (await hasCMake()) await buildCMake(null)
    else await buildSingle()
  } catch {}
  app.refreshProblems?.()
}

export async function buildForRun() {
  if (state.running || state.debugging || !state.root || !(await app.saveAll())) return null
  prepare()
  let exe
  try {
    if (await hasCMake()) {
      if (!(await ensureConfigured())) return null
      const name = state.target
      if (!name) {
        toast('This project has no executable target')
        return null
      }
      if (!(await buildCMake(name))) return null
      exe = state.targets.find((t) => t.name === name)?.path
    } else exe = await buildSingle()
  } catch {
    return null
  } finally {
    app.refreshProblems?.()
  }
  if (!exe) return null
  const c = runConfig()
  return { exe, cwd: c.cwd || ((await hasCMake()) ? state.build : state.root) }
}

async function doRun() {
  const b = await buildForRun()
  if (!b) return
  runOut.clear()
  try {
    const code = await runTask(runOut, b.exe, b.exe, parsedArgs(), b.cwd, { background: false, interactive: true, env: parsedEnv() })
    runOut.append(code === null ? '\nProcess finished (killed)\n' : `\nProcess finished with exit code ${code}\n`, 'hd')
    if (!$('work').hidden) ed.focus()
  } catch {}
}

async function clean() {
  if (state.running || !(await hasCMake())) return
  prepare()
  if (await ensureConfigured()) await runTask(buildOut, 'clean', 'cmake', ['--build', state.build, '--target', 'clean'], state.root)
}

async function ctest() {
  if (state.running || !(await app.saveAll()) || !(await hasCMake())) return
  prepare()
  if (!(await buildCMake(null))) return
  const code = await runTask(buildOut, 'ctest', 'ctest', ['--test-dir', state.build, '--output-on-failure', '-j', String(await jobCount())], state.root)
  buildOut.append(code === 0 ? 'All tests passed\n' : `Tests failed (${code})\n`, code === 0 ? 'ok' : 'er')
}

async function reload() {
  if (state.running || !state.root) return
  if (!(await hasCMake())) return toast('No CMakeLists.txt in this folder')
  prepare()
  await app.saveAll()
  configure().catch(() => {})
}

async function stop() {
  if (state.debugging) await app.debug.stop()
  else await invoke('task_kill')
}

$('build').addEventListener('click', doBuild)
$('run').addEventListener('click', doRun)
$('stop').addEventListener('click', stop)
$('stdin').addEventListener('keydown', (e) => {
  if (e.ctrlKey && e.key === 'c' && !getSelection().toString() && e.target.selectionStart === e.target.selectionEnd) {
    e.preventDefault()
    invoke('task_kill')
    return
  }
  if (e.key !== 'Enter') return
  const text = e.target.value
  e.target.value = ''
  runOut.append(text + '\n', 'in')
  invoke('task_stdin', { text: text + '\n' }).catch(toast)
})

reg('build', 'Build Project', 'C-F9', doBuild)
reg('run', 'Run', 'S-F10', doRun)
reg('stop', 'Stop', 'C-F2', stop)
app.rerun = doRun
app.stopTask = stop
reg('clean', 'Clean', null, clean)
reg('ctest', 'Run Tests (CTest)', null, ctest)
reg('reload-cmake', 'Reload CMake Project', null, reload)
reg('run-config', 'Edit Run Configuration…', null, editConfig)
