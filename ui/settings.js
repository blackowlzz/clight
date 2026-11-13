import * as theme from './theme.js'
import { h, app, state, settings, store, toast, modal, promptBox, seg } from './core.js'
import * as intel from './intel.js'
import { closeNote } from './notify.js'
import * as ai from './ai.js'
import * as presence from './presence.js'
import { PRESETS } from './ai.js'
import { invoke, guard } from './core.js'

const SECTIONS = [
  ['theme', 'Themes'],
  ['colors', 'Colors'],
  ['interface', 'Interface'],
  ['editor', 'Editor'],
  ['tools', 'Tools'],
  ['ai', 'AI'],
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
    row('Mouse glow', 'Auto turns it on, unless the system asks for less motion', pick('glow', [['auto', 'Auto'], ['on', 'On'], ['off', 'Off']])),
    row('Glow strength', '', slider('glowStrength', 1, 20, 1, '%')),
    row('Glow size', '', slider('glowRadius', 80, 500, 20, 'px')),
    row('Interface animations', 'Popups, panels, hover. Auto turns them on, unless the system asks for less motion', pick('fxUi', AUTO)),
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

function aiPage(rerender) {
  const save = (key, v) => {
    settings[key] = v
    store(key, v)
  }
  const chatRow = row('AI chat', 'A chat in a column on the right (Alt+8)', seg([[true, 'On'], [false, 'Off']], settings.aiChat, (v) => {
    save('aiChat', v)
    if (v) closeNote('ai')
    app.syncChat?.()
  }))
  const provider = row('Assistant', 'The small built-in AI needs no setup. Your own server is for the advanced chat', seg([['builtin', 'Small built-in AI'], ['custom', 'Advanced: my own server']], settings.aiProvider, (v) => {
    save('aiProvider', v)
    ai.refresh().then(() => rerender?.())
  }))
  if (ai.isBuiltin()) {
    const box = h('span', { class: 'ai-control' })
    const k = ai.size()
    const m = ai.MODELS[k]
    const paint = () => {
      if (!box.isConnected && box.dataset.ready) return window.removeEventListener('ai-changed', paint)
      box.dataset.ready = '1'
      box.replaceChildren(
        ai.installing()
          ? h('span', { class: 'muted' }, 'Installing…')
          : ai.isInstalled(k)
            ? h('span', {}, h('span', { class: 'muted' }, 'Installed  '), h('button', { onclick: () => guard(ai.removeBuiltin(k)) }, 'Remove'))
            : h('button', { class: 'primary', onclick: () => guard(ai.installBuiltin(k)) }, `Install (${m.download})`),
      )
    }
    window.addEventListener('ai-changed', paint)
    paint()
    ai.refresh().then(paint)
    const sizes = seg(Object.entries(ai.MODELS).map(([id, x]) => [id, `${x.label} · ${x.params}`]), k, (v) => {
      save('aiSize', v)
      ai.refresh().then(() => {
        window.dispatchEvent(new Event('ai-changed'))
        rerender?.()
      })
    })
    return h(
      'div',
      {},
      chatRow,
      provider,
      row('Size', 'You can install both and switch. Only one runs at a time', sizes),
      row('Smart suggestions', 'The AI finishes your lines in gray while you type. It uses the processor for a moment after you pause', seg([[true, 'On'], [false, 'Off']], settings.ghostAi, (v) => {
        save('ghostAi', v)
        if (!v) invoke('ai_suggest_cancel').catch(() => {})
        app.clearGhost?.()
      })),
      row(`${m.label} AI · Qwen2.5-Coder ${m.params}`, `${m.tag}. Download ${m.download}. Memory ${m.ram} while it runs. Processor: ${m.cpu}. Answers: ${m.answers}. These are estimates. It starts by itself and stops after 5 idle minutes.`, box),
      h('p', { class: 'muted', style: 'padding:6px 0' }, `It runs on your computer and your code is not sent anywhere. ${ai.advice()}`.trim()),
    )
  }
  const field = (key, placeholder, type = 'text', extra = {}) =>
    h('input', { class: 'wide', type, value: settings[key], placeholder, spellcheck: 'false', autocomplete: 'off', oninput: (e) => save(key, e.target.value.trim()), ...extra })
  const urlInput = field('aiUrl', 'http://127.0.0.1:11434/v1')
  const models = h('datalist', { id: 'ai-models' })
  const modelInput = field('aiModel', 'Model name', 'text', { list: 'ai-models' })
  const keyInput = field('aiKey', 'Optional', 'password')
  const result = h('span', { class: 'muted' })
  const current = PRESETS.find((p) => p[2] === settings.aiUrl)?.[0] ?? 'custom'
  const presets = seg([...PRESETS.map(([id, label]) => [id, label]), ['custom', 'Custom']], current, (v) => {
    const p = PRESETS.find((x) => x[0] === v)
    if (!p) return urlInput.focus()
    urlInput.value = p[2]
    save('aiUrl', p[2])
  })
  const test = h('button', {
    onclick: async () => {
      result.textContent = 'Connecting…'
      try {
        const list = await invoke('ai_models', { endpoint: { url: settings.aiUrl, key: settings.aiKey, model: '' } })
        models.replaceChildren(...list.map((m) => h('option', { value: m })))
        if (list.length === 1 && !settings.aiModel) {
          modelInput.value = list[0]
          save('aiModel', list[0])
        }
        result.textContent = list.length ? `Connected. ${list.length} model${list.length === 1 ? '' : 's'} found` : 'Connected, but the server lists no models'
      } catch (e) {
        result.textContent = String(e)
      }
    },
  }, 'Test connection')
  return h(
    'div',
    {},
    chatRow,
    provider,
    row('Server', 'Ollama, LM Studio and llama.cpp run on your own computer', presets),
    row('Address', 'Any server that speaks the OpenAI API', urlInput),
    row('Model', 'Test the connection to see the names', h('span', {}, modelInput, models)),
    row('API key', 'Only for online services. It stays on this computer', keyInput),
    h('div', { class: 'set-actions' }, test, result),
    h('p', { class: 'muted', style: 'padding:6px 0' }, 'Your code leaves this computer only if you point clight to an online service.'),
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
    opt('trimSave', 'Removes spaces at the end of lines when a file is saved', 'Trim trailing spaces', onOff),
    opt('newlineSave', 'Makes sure the file ends with a new line when it is saved', 'Final new line', onOff),
    opt('updates', 'At startup, from GitHub', 'Check for updates', onOff),
    opt('discord', 'Shows the file and project you are working on in your Discord profile, if Discord is open', 'Discord status', onOff, () => presence.sync()),
    opt('termBar', 'A separate command field, or typing straight into the shell', 'Terminal input', [[true, 'Command bar'], [false, 'Classic']], () => app.term?.syncBar()),
  )
}

const PAGES = { theme: themePage, colors: colorsPage, interface: interfacePage, editor: editorPage, tools: toolsPage, ai: aiPage }

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

