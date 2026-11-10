import * as theme from './theme.js'
import { h, app, state, settings, store, toast, modal, promptBox, seg } from './core.js'
import * as intel from './intel.js'

const SECTIONS = [
  ['theme', 'Themes'],
  ['colors', 'Colors'],
  ['interface', 'Interface'],
  ['editor', 'Editor'],
  ['tools', 'Tools'],
]

function refresh(rerender) {
  theme.apply()
  app.applyLook?.()
  rerender?.()
}

const AUTO = [['auto', 'Auto'], ['on', 'On'], ['off', 'Off']]

const row = (title, sub, control) => h('div', { class: 'set-row' }, h('div', {}, title, sub ? h('span', { class: 'sub' }, sub) : null), control)

function slider(key, min, max, step, unit, cb) {
  const value = h('span', { class: 'sv' }, theme.look[key] + unit)
  const input = h('input', {
    type: 'range', min, max, step, value: theme.look[key],
    oninput: (e) => {
      theme.look[key] = Number(e.target.value)
      value.textContent = theme.look[key] + unit
      theme.save()
      refresh()
      cb?.()
    },
  })
  return h('div', { class: 'slider' }, input, value)
}

const pick = (key, options, cb) =>
  seg(options, theme.look[key], (v) => {
    theme.look[key] = v
    theme.save()
    refresh(cb)
  })

function themeCard(t, rerender) {
  const c = { ...t.colors }
  const on = theme.look.themeId === t.id
  const line = (color, width) => {
    const l = h('span', { class: 'tl' })
    l.style.background = color
    l.style.width = width
    return l
  }
  const mini = h('span', { class: 'tmini' }, h('span', { class: 'tside' }), h('span', { class: 'tcode' }, line(c.k, '38%'), line(c.f ?? c.text, '62%'), line(c.r ?? c.muted, '48%'), line(c.c ?? c.faint, '30%')))
  mini.style.background = c.bg
  mini.firstChild.style.background = c.chrome
  const card = h(
    'button',
    {
      class: 'tcard' + (on ? ' on' : ''),
      'data-id': t.id,
      onclick: () => {
        theme.setTheme(t.id)
        refresh(rerender)
        app.requestSemanticAll?.()
      },
    },
    mini,
    h('span', { class: 'tname' }, t.name),
  )
  if (t.custom) {
    card.append(
      h(
        'span',
        {
          class: 'tdel',
          title: 'Delete theme',
          onclick: (e) => {
            e.stopPropagation()
            theme.deleteCustom(t.id)
            refresh(rerender)
          },
        },
        '×',
      ),
    )
  }
  return card
}

function themePage(rerender) {
  const copy = () => {
    app.copyText(theme.exportTheme())
    toast('Theme copied to the clipboard')
  }
  const paste = async () => {
    const text = await promptBoxLarge()
    if (!text) return
    try {
      theme.importTheme(text)
      refresh(rerender)
      app.requestSemanticAll?.()
    } catch (e) {
      toast(e.message ?? e)
    }
  }
  return h(
    'div',
    {},
    h('div', { class: 'tgrid' }, ...theme.themes().map((t) => themeCard(t, rerender))),
    h(
      'div',
      { class: 'set-actions' },
      h(
        'button',
        {
          onclick: async () => {
            const name = await promptBox('Save current colors as', theme.currentBase().name + ' Custom', 'Save')
            if (name) {
              theme.saveCustom(name.slice(0, 40))
              refresh(rerender)
            }
          },
        },
        'Save as New Theme…',
      ),
      h('button', { onclick: copy }, 'Export'),
      h('button', { onclick: paste }, 'Import…'),
    ),
  )
}

function promptBoxLarge() {
  return new Promise((resolve) => {
    let close
    const area = h('textarea', { class: 'field', style: 'height:180px', spellcheck: 'false', placeholder: 'Paste the theme here' })
    const done = (v) => {
      close()
      resolve(v)
    }
    close = modal(h('div', { class: 'dialog' }, h('h3', {}, 'Import theme'), area, h('div', { class: 'actions' }, h('button', { onclick: () => done(null) }, 'Cancel'), h('button', { class: 'primary', onclick: () => done(area.value) }, 'Import'))))
    area.focus()
  })
}

function colorsPage(rerender) {
  const reset = h(
    'button',
    {
      disabled: !theme.isCustomized(),
      onclick: () => {
        theme.resetColors()
        refresh(rerender)
      },
    },
    'Reset to Theme',
  )
  const groups = theme.TOKENS.map(([group, list]) =>
    h(
      'div',
      { class: 'cgroup' },
      h('div', { class: 'eyebrow' }, group),
      ...list.map(([key, label]) => {
        const current = theme.colors()[key]
        const swatch = h('label', { class: 'swatch' })
        swatch.style.background = current
        const text = h('input', { class: 'hex', value: current, maxlength: 7, spellcheck: 'false' })
        const commit = (v) => {
          theme.setColor(key, v)
          swatch.style.background = v
          reset.disabled = false
          refresh()
          if (['k', 'f', 'y', 'r'].includes(key)) app.requestSemanticAll?.()
        }
        const input = h('input', { type: 'color', value: current, oninput: (e) => ((text.value = e.target.value), commit(e.target.value)), 'aria-label': label })
        swatch.append(input)
        text.addEventListener('input', () => /^#[0-9a-f]{6}$/i.test(text.value) && ((input.value = text.value), commit(text.value)))
        return h('div', { class: 'crow' }, swatch, h('span', { class: 'cn' }, label), text)
      }),
    ),
  )
  return h(
    'div',
    {},
    h('div', { class: 'cgrid' }, ...groups),
    h(
      'div',
      { class: 'set-actions' },
      reset,
      h('span', { class: 'sub' }, `Editing ${theme.currentBase().name}`),
    ),
  )
}

function interfacePage(rerender) {
  return h(
    'div',
    {},
    row('Font', '', pick('uiFont', [['inter', 'Inter Tight'], ['system', 'System']])),
    row('Text size', '', slider('uiSize', 11, 16, 0.5, 'px')),
    row('Corners', '0 is fully square', slider('radius', 0, 10, 1, 'px')),
    row('Density', '', pick('density', [['compact', 'Compact'], ['normal', 'Normal'], ['comfortable', 'Roomy']])),
    row('Mouse glow', 'Auto turns it off on weak machines', pick('glow', [['auto', 'Auto'], ['on', 'On'], ['off', 'Off']])),
    row('Glow strength', '', slider('glowStrength', 1, 20, 1, '%')),
    row('Glow size', '', slider('glowRadius', 80, 500, 20, 'px')),
    row('Interface animations', 'Popups, panels, hover. Auto turns them off on weak machines', pick('fxUi', AUTO)),
    row('Smooth caret', 'The caret glides to its new position', pick('fxCaret', AUTO)),
    row('Caret motion blur', 'A short streak behind the caret as it moves', pick('fxTrail', AUTO)),
    row('Typing effects', 'A soft glow that follows the caret, a pulse on matching brackets, and a fade on new lines and saves', pick('fxType', AUTO)),
    h(
      'div',
      { class: 'set-actions' },
      h(
        'button',
        {
          onclick: () => {
            theme.resetAll()
            refresh(rerender)
            app.requestSemanticAll?.()
          },
        },
        'Reset All Appearance',
      ),
    ),
  )
}

function editorPage(rerender) {
  const b = theme.currentBase()
  const flag = (key, fallback, label) =>
    row(
      label,
      '',
      seg([[true, 'On'], [false, 'Off']], theme.look[key] ?? fallback ?? false, (v) => {
        theme.look[key] = v
        theme.save()
        refresh()
      }),
    )
  const customInput = h('input', {
    value: theme.look.edCustom,
    placeholder: 'Installed font name',
    spellcheck: 'false',
    oninput: (e) => {
      theme.look.edCustom = e.target.value
      theme.save()
      refresh()
    },
  })
  return h(
    'div',
    {},
    row('Font', '', pick('edFont', [['jetbrains', 'JetBrains Mono'], ['system', 'System'], ['custom', 'Other']], rerender)),
    theme.look.edFont === 'custom' ? row('Font name', '', customInput) : null,
    row('Font size', '', slider('edSize', 10, 24, 1, 'px')),
    row('Line height', '', slider('lineHeight', 1.2, 2, 0.05, '')),
    row('Caret', '', pick('caret', [['bar', 'Bar'], ['block', 'Block'], ['underline', 'Underline']])),
    flag('bold', b.bold, 'Bold keywords'),
    flag('italic', b.italic, 'Italic comments'),
    row('Line numbers', '', pick('nums', [[true, 'On'], [false, 'Off']])),
    row('Right margin', 'A guide line at this column', pick('rightMargin', [[0, 'Off'], [80, '80'], [100, '100'], [120, '120']])),
    row('Indent guides', '', pick('guides', [[true, 'On'], [false, 'Off']])),
    row('Current line highlight', '', pick('curline', [[true, 'On'], [false, 'Off']])),
  )
}

function toolsPage() {
  const opt = (key, sub, title, options, cb) =>
    row(
      title,
      sub,
      seg(options, settings[key], (v) => {
        settings[key] = v
        store(key, v)
        cb?.(v)
      }),
    )
  const triple = [['auto', 'Auto'], ['on', 'On'], ['off', 'Off']]
  const onOff = [[true, 'On'], [false, 'Off']]
  return h(
    'div',
    {},
    opt('intel', 'Auto starts clangd only with 1.2 GB of RAM free', 'Code intelligence', triple, async (v) => {
      if (!state.root) return
      if (v === 'off') await intel.stopIntel()
      else if (state.lsp !== 'on') intel.startIntel()
    }),
    opt('index', 'Needed for usages and rename across files. Auto: 3 GB free', 'Project index', triple, () => state.lsp === 'on' && intel.restartIntel()),
    opt('formatStyle', 'Used unless the project has its own .clang-format', 'Code style', [['clean', 'Clean'], ['allman', 'Allman'], ['google', 'Google'], ['llvm', 'LLVM']]),
    opt('ghost', 'Gray text after the caret that finishes the line from code you already wrote. Tab accepts it', 'Line suggestions', onOff, (v) => !v && app.clearGhost?.()),
    opt('autosave', 'When the window loses focus', 'Save automatically', onOff),
    opt('updates', 'At startup, from GitHub', 'Check for updates', onOff),
    opt('termBar', 'A separate command field, or typing straight into the shell', 'Terminal input', [[true, 'Command bar'], [false, 'Classic']], () => app.term?.syncBar()),
  )
}

const PAGES = { theme: themePage, colors: colorsPage, interface: interfacePage, editor: editorPage, tools: toolsPage }

export function customizeView(initial = 'theme') {
  let section = initial
  const nav = h('div', { class: 'set-nav' })
  const body = h('div', { class: 'set-body' })
  const render = () => {
    nav.replaceChildren(
      ...SECTIONS.map(([id, label]) =>
        h(
          'button',
          {
            class: id === section ? 'on' : '',
            onclick: () => {
              section = id
              render()
            },
          },
          label,
        ),
      ),
    )
    body.replaceChildren(PAGES[section](render))
  }
  render()
  return h('div', { class: 'set' }, nav, body)
}

export function open(initial) {
  let close
  close = modal(h('div', { class: 'settings' }, h('div', { class: 'set-head' }, h('h3', {}, 'Settings'), h('button', { class: 'iconbtn', title: 'Close', onclick: () => close() }, '×')), customizeView(initial)))
}

