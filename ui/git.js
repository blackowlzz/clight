import { $, h, invoke, app, state, reg, toast, guard, confirmBox, promptBox, showMenu, basename, seg, iconBtn, palette, fuzzy } from './core.js'
import { runTask, buildOut } from './build.js'

const view = $('v-git')
let mode = 'changes'
let log = null
let amend = false
const message = h('textarea', { class: 'field', placeholder: 'Commit message', spellcheck: 'false' })

const isStaged = (f) => f.x !== ' ' && f.x !== '?'
const untracked = (f) => f.x === '?'
const letter = (f) => (f.x === '?' ? 'U' : (f.y !== ' ' ? f.y : f.x))

export async function refresh() {
  if (!state.root) return
  state.git = await invoke('git_status', { root: state.root })
  const b = $('st-git')
  if (state.git) {
    const { branch, ahead, behind } = state.git
    b.textContent = branch + (ahead ? ` ↑${ahead}` : '') + (behind ? ` ↓${behind}` : '')
  } else b.textContent = ''
  app.renderTree?.()
  if (app.isShown?.('git')) render()
}

export function fileStatus(path) {
  if (!state.git || !state.root || !path.startsWith(state.root)) return null
  const rel = path.slice(state.root.length + 1)
  return state.git.files.find((f) => f.path === rel || f.path === rel + '/') ?? null
}

async function openDiff(f) {
  const text = await invoke('git_diff', { root: state.root, path: f.path, untracked: untracked(f) })
  await app.openVirtual(`diff:${f.path}`, `${basename(f.path.replace(/\/$/, ''))} (diff)`, text || 'No differences\n')
}

async function toggleStage(f, on) {
  try {
    await invoke(on ? 'git_stage' : 'git_unstage', { root: state.root, paths: [f.path] })
  } catch (e) {
    toast(e)
  }
  await refresh()
}

async function discard(f) {
  const yes = await confirmBox(`Roll back ${basename(f.path)}?`, untracked(f) ? 'This new file will be deleted.' : 'Your changes to this file will be lost.', 'Roll back')
  if (!yes) return
  await invoke('git_discard', { root: state.root, path: f.path, untracked: untracked(f) })
  await app.afterExternalChange()
  await refresh()
}

async function commit(push) {
  const msg = message.value.trim()
  if (!msg) return toast('Write a commit message')
  await app.saveAll()
  if (!state.git.files.some(isStaged)) await invoke('git_stage', { root: state.root, paths: state.git.files.map((f) => f.path) })
  await invoke('git_commit', { root: state.root, message: msg, amend })
  message.value = ''
  amend = false
  log = null
  toast('Committed')
  await refresh()
  await app.afterExternalChange()
  if (push) await remote('push')
}

async function remote(what) {
  if (state.running) return toast('Another task is running')
  buildOut.clear()
  await runTask(buildOut, `git ${what}`, 'git', [what], state.root)
  await refresh()
  await app.afterExternalChange()
}

async function render() {
  if (!state.git) {
    view.replaceChildren(
      h('div', { class: 'notice' }, 'This folder is not a Git repository.'),
      h('div', { style: 'padding:0 10px' }, h('button', { onclick: () => guard(runTask(buildOut, 'git init', 'git', ['init'], state.root).then(refresh)) }, 'Create repository')),
    )
    return
  }
  const bar = h(
    'div',
    { class: 'gitbar' },
    seg([['changes', 'Changes'], ['log', 'Log']], mode, (m) => {
      mode = m
      render()
    }),
  )
  if (mode === 'log') {
    log ??= await invoke('git_log', { root: state.root }).catch(() => [])
    view.replaceChildren(
      bar,
      h(
        'div',
        { class: 'grow', style: 'overflow:auto' },
        h(
          'div',
          { class: 'log' },
          ...log.map((c) =>
            h(
              'div',
              {
                class: 'lg',
                title: c.subject,
                onclick: () => guard(invoke('git_show', { root: state.root, hash: c.hash }).then((t) => app.openVirtual(`show:${c.hash}`, c.hash, t))),
              },
              h('div', {}, c.subject),
              h('div', {}, `${c.hash} · ${c.author} · ${c.when}`),
            ),
          ),
        ),
      ),
    )
    return
  }
  const rows = state.git.files.map((f) =>
    h(
      'div',
      { class: 'row', title: f.path, onclick: () => guard(openDiff(f)) },
      h('input', { type: 'checkbox', checked: isStaged(f), onclick: (e) => (e.stopPropagation(), toggleStage(f, e.target.checked)) }),
      h('span', { class: 'nm' }, basename(f.path.replace(/\/$/, ''))),
      h('span', { class: 'st' }, letter(f)),
      h('span', { class: 'rm', title: 'Roll back', onclick: (e) => (e.stopPropagation(), guard(discard(f))) }, '↺'),
    ),
  )
  const amendBtn = h(
    'button',
    {
      class: 'chip',
      'aria-pressed': String(amend),
      onclick: () => {
        amend = !amend
        render()
      },
    },
    'Amend',
  )
  view.replaceChildren(
    bar,
    h('div', { class: 'gitlist grow', style: 'overflow:auto' }, ...(rows.length ? rows : [h('div', { class: 'notice' }, 'No changes')])),
    h('div', { class: 'commit' }, message, h('div', { class: 'acts' }, h('button', { class: 'primary', onclick: () => guard(commit(false)) }, 'Commit'), h('button', { onclick: () => guard(commit(true)) }, 'Commit and Push'), amendBtn)),
  )
}

export const shown = () => guard(refresh())
export const panelActions = () =>
  h(
    'span',
    { class: 'headacts' },
    h('button', { class: 'plain', onclick: () => guard(remote('pull')) }, 'Pull'),
    h('button', { class: 'plain', onclick: () => guard(remote('push')) }, 'Push'),
    iconBtn('refresh', 'Refresh', () => guard(refresh())),
  )

async function stash(pop) {
  await app.saveAll()
  const out = await invoke('git_stash', { root: state.root, pop })
  toast(String(out).split('\n')[0] || (pop ? 'Changes restored' : 'Changes stashed'))
  await refresh()
  await app.afterExternalChange()
}

function pickBranch(title, list, run) {
  const others = list.filter((b) => b !== state.git?.branch)
  palette({
    placeholder: title,
    query: async (q) => others.filter((b) => !q || fuzzy(q, b) >= 0).map((b) => ({ b, open: () => guard(run(b)) })),
    render: ({ b }) => h('div', {}, h('b', {}, b)),
  })
}

async function branchMenu() {
  if (!state.git) return
  const list = await invoke('git_branches', { root: state.root })
  const r = $('st-git').getBoundingClientRect()
  const sw = async (name, create) => {
    await app.saveAll()
    await invoke('git_checkout', { root: state.root, name, create }).catch(toast)
    await refresh()
    await app.afterExternalChange()
  }
  showMenu(r.left, r.bottom + 2, [
    ...list.map((b) => [b, () => guard(sw(b, false)), b === state.git.branch ? 'current' : '']),
    '-',
    ['New branch…', async () => {
      const name = await promptBox('New branch', '', 'Create')
      if (name) guard(sw(name, true))
    }],
    ['Merge Branch…', () => pickBranch('Merge into the current branch', list, async (b) => {
      await app.saveAll()
      toast(String(await invoke('git_merge', { root: state.root, name: b })).split('\n')[0] || 'Merged')
      await refresh()
      await app.afterExternalChange()
    })],
    ['Delete Branch…', () => pickBranch('Delete a branch', list, async (b) => {
      if (await confirmBox(`Delete ${b}?`, 'Git only deletes it when it is merged.', 'Delete')) {
        await invoke('git_delete_branch', { root: state.root, name: b })
        toast(`Deleted ${b}`)
      }
    })],
    '-',
    ['Stash Changes', () => guard(stash(false))],
    ['Unstash (Pop)', () => guard(stash(true))],
  ])
}
$('st-git').addEventListener('click', () => guard(branchMenu()))

reg('commit', 'Commit…', 'C-k', () => app.showLeft('git', true))
reg('git-pull', 'Pull', null, () => remote('pull'))
reg('git-push', 'Push', 'C-S-k', () => remote('push'))
reg('git-branch', 'Branches…', null, branchMenu)
