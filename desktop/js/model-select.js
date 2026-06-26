function escapeHtml(text) {
  return String(text)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function isIconUrl(icon) {
  if (!icon) return false;
  const t = icon.trim();
  return t.startsWith('http://') || t.startsWith('https://') || t.startsWith('/');
}

function renderIcon(icon, label, className = 'model-select-icon') {
  if (!icon || !icon.trim()) {
    return `<span class="${className} model-select-icon-fallback" aria-hidden="true">◇</span>`;
  }
  if (isIconUrl(icon)) {
    return `<img class="${className}" src="${escapeHtml(icon)}" alt="" />`;
  }
  return `<span class="${className} model-select-icon-emoji" aria-hidden="true">${escapeHtml(icon)}</span>`;
}

function resolveDefaultModelId(models, preferred) {
  if (preferred && models.some((m) => m.id === preferred)) return preferred;
  const auto = models.find((m) => m.id === 'Auto' || m.id === 'auto');
  if (auto) return auto.id;
  return models[0]?.id || '';
}

export function mountModelSelect(rootId, { onChange, placeholder = 'Select model' } = {}) {
  const root = document.getElementById(rootId);
  if (!root) return null;

  const state = {
    models: [],
    value: '',
    open: false,
    placeholder,
    onChange,
  };

  root.classList.add('model-select');
  root.innerHTML = `
    <button type="button" class="model-select-trigger" aria-haspopup="listbox" aria-expanded="false">
      <span class="model-select-trigger-inner"></span>
      <svg class="model-select-chevron" viewBox="0 0 24 24" aria-hidden="true"><path d="M6 9l6 6 6-6" fill="none" stroke="currentColor" stroke-width="2"/></svg>
    </button>
    <div class="model-select-menu" role="listbox" hidden></div>
  `;

  const trigger = root.querySelector('.model-select-trigger');
  const triggerInner = root.querySelector('.model-select-trigger-inner');
  const menu = root.querySelector('.model-select-menu');

  function selectedModel() {
    return state.models.find((m) => m.id === state.value);
  }

  function renderTrigger() {
    const model = selectedModel();
    if (!model) {
      triggerInner.innerHTML = `<span class="model-select-label">${escapeHtml(state.placeholder)}</span>`;
      return;
    }
    const label = model.name || model.id;
    triggerInner.innerHTML = `
      ${renderIcon(model.icon, label)}
      <span class="model-select-label">${escapeHtml(label)}</span>
    `;
  }

  function renderMenu() {
    if (!state.models.length) {
      menu.innerHTML = `<div class="model-select-empty">${escapeHtml(state.placeholder)}</div>`;
      return;
    }
    menu.innerHTML = state.models
      .map((m) => {
        const label = m.name || m.id;
        const active = m.id === state.value ? ' active' : '';
        return `
          <button type="button" class="model-select-option${active}" role="option" data-value="${escapeHtml(m.id)}" aria-selected="${m.id === state.value}">
            ${renderIcon(m.icon, label, 'model-select-option-icon')}
            <span class="model-select-option-label">${escapeHtml(label)}</span>
          </button>
        `;
      })
      .join('');
  }

  function setOpen(open) {
    state.open = open;
    trigger.setAttribute('aria-expanded', open ? 'true' : 'false');
    menu.hidden = !open;
    root.classList.toggle('open', open);
    if (open) renderMenu();
  }

  function setValue(value, fireChange = false) {
    state.value = value;
    renderTrigger();
    if (fireChange) state.onChange?.(value);
  }

  trigger.addEventListener('click', (e) => {
    e.stopPropagation();
    setOpen(!state.open);
  });

  menu.addEventListener('click', (e) => {
    const btn = e.target.closest('.model-select-option');
    if (!btn) return;
    const value = btn.getAttribute('data-value');
    if (!value) return;
    setValue(value, true);
    setOpen(false);
  });

  document.addEventListener('mousedown', (e) => {
    if (!root.contains(e.target)) setOpen(false);
  });

  return {
    setModels(models, selectedId) {
      state.models = Array.isArray(models) ? models : [];
      const value = resolveDefaultModelId(state.models, selectedId);
      setValue(value, false);
      if (state.open) renderMenu();
    },
    getValue() {
      return state.value;
    },
    setValue(value, fireChange = false) {
      setValue(value, fireChange);
    },
    setPlaceholder(text) {
      state.placeholder = text || '';
      renderTrigger();
      if (state.open) renderMenu();
    },
    destroy() {
      root.innerHTML = '';
      root.classList.remove('model-select', 'open');
    },
  };
}
