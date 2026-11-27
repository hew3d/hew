// Styles for the Library's sources UI (docs/design/shared-library-sources.md
// 7.5): the sidebar block, the catalog, the source view, the Add dialog, the
// bar and the review. Injected beside LIBRARY_CSS by LibraryDialog; every
// color and size resolves through the same theme tokens.

// oxlint-disable-next-line lingui/no-unlocalized-strings -- stylesheet text, not copy
export const SOURCES_CSS = `
.hwlib__sources {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding-bottom: 8px;
}
.hwlib__src-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
  padding: 6px 8px;
  background: none;
  border: none;
  border-radius: var(--radius-panel-item);
  color: var(--text-secondary);
  font-family: var(--font-family-ui);
  font-size: var(--font-size-tool-row);
  text-align: left;
  cursor: pointer;
}
.hwlib__src-row:hover { background: var(--accent-tint-15); }
.hwlib__src-row--active { background: var(--accent-tint-15); color: var(--text-primary); }
.hwlib__src-row-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.hwlib__src-marks { display: flex; gap: 4px; flex: 0 0 auto; font-size: 11px; }
.hwlib__src-mark--followed { color: var(--accent-text-on-tint); }
.hwlib__src-mark--warning { color: var(--warning-text, #e0b341); }
.hwlib__src-action { color: var(--accent-text-on-tint); }
.hwlib__src-off { padding: 6px 8px; font-size: 11.5px; color: var(--text-faint); line-height: 1.4; }
.hwlib__src-link { color: var(--accent-base); text-decoration: underline; }

.hwlib__src-center { display: flex; flex-direction: column; flex: 1 1 auto; min-width: 0; min-height: 0; }
.hwlib__src-head {
  display: flex; flex-direction: column; gap: 6px;
  padding: 12px 14px 10px;
  border-bottom: 1px solid var(--border-hairline);
  flex: 0 0 auto;
}
.hwlib__src-title { font-size: 15px; font-weight: 600; color: var(--text-primary); margin: 0; }
.hwlib__src-by { font-size: 12px; color: var(--text-muted); }
.hwlib__src-meta { display: flex; flex-wrap: wrap; gap: 4px 12px; font-size: 11.5px; color: var(--text-muted); }
.hwlib__src-desc { font-size: 12px; color: var(--text-secondary); line-height: 1.4; margin: 0; }
.hwlib__src-buttons { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
.hwlib__src-buttons button {
  padding: 5px 12px;
  border-radius: var(--radius-control);
  font-family: var(--font-family-ui);
  font-size: 12px;
  cursor: pointer;
}
.hwlib__src-buttons button:disabled, .hwlib__src-dialog button:disabled { opacity: 0.5; cursor: default; }
.hwlib__src-warning {
  padding: 6px 8px; border-radius: var(--radius-control);
  border: 1px solid var(--warning-border, #8a6d1f);
  color: var(--warning-text, #e0b341);
  font-size: 11.5px; line-height: 1.4;
}
.hwlib__src-progress { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-secondary); }
.hwlib__src-progress progress { flex: 1 1 auto; min-width: 60px; }
.hwlib__src-summary { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-primary); }
.hwlib__src-search { margin-top: 2px; }
.hwlib__src-grid { flex: 1 1 auto; min-height: 0; overflow-y: auto; outline: none; }
.hwlib__src-empty { padding: 20px; text-align: center; color: var(--text-faint); font-size: 12.5px; }
.hwlib__row-state { margin-left: 8px; font-size: 10.5px; color: var(--text-faint); white-space: nowrap; }

.hwlib__catalog { display: flex; flex-direction: column; flex: 1 1 auto; min-width: 0; min-height: 0; overflow-y: auto; padding: 12px 14px; gap: 8px; }
.hwlib__catalog-row {
  display: flex; align-items: flex-start; gap: 12px;
  padding: 10px 12px;
  border: 1px solid var(--border-panel);
  border-radius: var(--radius-control);
  background: none; text-align: left; cursor: pointer;
  font-family: var(--font-family-ui); color: var(--text-secondary);
}
.hwlib__catalog-row:hover { background: var(--accent-tint-15); }
.hwlib__catalog-main { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1 1 auto; }
.hwlib__catalog-name { font-size: 13px; font-weight: 600; color: var(--text-primary); }
.hwlib__catalog-sub { font-size: 11.5px; color: var(--text-muted); }
.hwlib__catalog-add {
  flex: 0 0 auto; padding: 5px 12px; border-radius: var(--radius-control);
  font-family: var(--font-family-ui); font-size: 12px; cursor: pointer;
}

.hwlib__src-overlay {
  position: fixed; inset: 0; z-index: 2100;
  display: flex; align-items: center; justify-content: center;
  background: var(--backdrop-dim, rgba(0,0,0,0.6));
}
.hwlib__src-dialog {
  display: flex; flex-direction: column; gap: 10px;
  width: min(460px, 92vw); max-height: 80vh; overflow-y: auto;
  padding: 18px 20px;
  background: var(--surface-overlay, #2a2a2a);
  border: 1px solid var(--border-strong, #4a4a4a);
  border-radius: var(--radius-control, 6px);
  box-shadow: var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6));
  font-family: var(--font-family-ui); font-size: 13px; color: var(--text-secondary);
}
.hwlib__src-dialog h2 { margin: 0; font-size: 15px; color: var(--text-primary); }
.hwlib__src-dialog input[type='text'], .hwlib__src-dialog input[type='url'] {
  box-sizing: border-box; width: 100%; padding: 6px 9px;
  background: var(--surface-input); border: 1px solid var(--border-panel);
  border-radius: var(--radius-control); color: var(--text-primary);
  font-family: var(--font-family-ui); font-size: 12.5px;
}
.hwlib__src-dialog-actions { display: flex; justify-content: flex-end; gap: 8px; }
.hwlib__src-dialog-actions button {
  padding: 5px 14px; border-radius: var(--radius-control);
  font-family: var(--font-family-ui); font-size: 13px; cursor: pointer;
}
.hwlib__src-facts { display: grid; grid-template-columns: auto 1fr; gap: 3px 12px; margin: 0; font-size: 12.5px; }
.hwlib__src-facts dt { color: var(--text-muted); }
.hwlib__src-facts dd { margin: 0; color: var(--text-primary); }

.hwlib__src-bar {
  display: flex; align-items: center; gap: 12px;
  padding: 8px 16px;
  background: var(--surface-overlay, #2a2a2a);
  border-bottom: 1px solid var(--border-strong, #4a4a4a);
  color: var(--text-primary); font-size: 13px; flex: 0 0 auto;
}
.hwlib__src-bar-lines { flex: 1 1 auto; display: flex; flex-direction: column; gap: 2px; }
.hwlib__src-bar button {
  padding: 4px 14px; border-radius: var(--radius-control);
  font-family: var(--font-family-ui); font-size: 13px; cursor: pointer;
}
.hwlib__src-review-row {
  display: flex; gap: 10px; align-items: flex-start;
  padding: 8px 10px;
  border: 1px solid var(--border-panel);
  border-radius: var(--radius-control);
}
.hwlib__src-review-row--warn { border-color: var(--warning-border, #8a6d1f); }
.hwlib__src-review-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1 1 auto; font-size: 13px; }
.hwlib__src-review-sub { font-size: 12px; color: var(--text-tertiary, #ccc); }
.hwlib__src-review-warn { font-size: 12px; color: var(--warning-text, #e0b341); }
.hwlib__src-review-result { font-size: 12px; color: var(--text-primary); }
.hwlib__src-review-result--failed { color: var(--danger-base); }
.hwlib__src-review-choices { display: flex; gap: 6px; margin-top: 4px; }
.hwlib__src-review-choices button {
  padding: 3px 10px; border-radius: var(--radius-control);
  font-family: var(--font-family-ui); font-size: 12px; cursor: pointer;
}
.hwlib__src-state { font-size: 11px; font-weight: 500; color: var(--accent-text-on-tint); }
.hwlib__src-state--warn { color: var(--warning-text, #e0b341); }
.hwlib__src-from { font-size: 11.5px; color: var(--text-secondary); }
`
