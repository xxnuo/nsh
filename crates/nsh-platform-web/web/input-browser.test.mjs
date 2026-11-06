import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';

const chromium = [process.env.CHROME, 'chromium', 'chromium-browser', 'google-chrome',
  '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'].filter(Boolean).find((name) => {
  try { execFileSync(name, ['--version'], { stdio: 'ignore' }); return true; } catch { return false; }
});

test('editable DOM controls own accessible names, descriptions and disabled state through updates',
  { skip: !chromium }, async () => {
    const dir = await mkdtemp(join(tmpdir(), 'nsh-input-browser-'));
    try {
      const source = await readFile(new URL('./nsh.js', import.meta.url), 'utf8');
      const module = 'data:text/javascript;base64,' + Buffer.from(source + '\nexport { Player };').toString('base64');
      await writeFile(join(dir, 'index.html'), `<!doctype html><meta charset="utf-8"><body>
        <script type="module">
        import { Player } from '${module}';
        const check = (ok, message) => { if (!ok) throw Error(message); };
        try {
          const player = Object.create(Player.prototype);
          player.views = new Map(); player.event = () => {}; player.call = () => {};
          for (const propsFirst of [true, false]) {
            const wrapper = document.createElement('div');
            Object.assign(wrapper, { _id: 2, _events: new Set(), _pressable: true });
            document.body.append(wrapper); player.views.set(2, wrapper);
            let config = { multiline: false, editable: true, placeholder: 'ws://localhost:6767', autoCapitalize: 'none' };
            let props = { label: '主机地址', hint: '请输入开发主机地址', focusable: true };
            const input = () => player.op_in(['in', 2, config, null]);
            const update = () => player.op_vp(['vp', 2, props]);
            if (propsFirst) { update(); input(); } else { input(); update(); }
            let field = wrapper._field;
            const named = (name, hint) => {
              check(field.getAttribute('aria-label') === name, 'actual field lost accessible name: ' + name);
              check(field.getAttribute('aria-description') === hint, 'actual field lost description: ' + hint);
              check(!wrapper.hasAttribute('aria-label') && !wrapper.hasAttribute('aria-description') && !wrapper.hasAttribute('role'), 'wrapper duplicates field accessibility');
              check(!wrapper.hasAttribute('tabindex'), 'wrapper creates a second Tab stop');
            };
            named('主机地址', '请输入开发主机地址');
            check(!field.hasAttribute('role'), 'native textbox role must not become the pressable wrapper button');
            field.focus(); check(document.activeElement === field, 'enabled input cannot focus');
            field.blur(); player.op_cmd(['cmd', 2, 'a11yFocus']);
            check(document.activeElement === field && !wrapper.hasAttribute('tabindex'), 'accessibility focus targets the layout wrapper');
            player.op_al(['al', 2, '远程主机']); named('远程主机', '请输入开发主机地址');
            props = { ...props, label: '远程主机', hint: '地址已更新', role: 'combobox', expanded: true };
            update(); named('远程主机', '地址已更新');
            check(field.getAttribute('role') === 'combobox' && field.getAttribute('aria-expanded') === 'true', 'field lost explicit role or state');
            props = { ...props, role: null, expanded: null, disabled: true };
            update();
            check(field.disabled && field.matches(':disabled') && field.getAttribute('aria-disabled') === 'true', 'disabled did not reach the native control');
            check(!field.readOnly, 'disabled must not change the editable setting');
            field.blur(); field.focus(); check(document.activeElement !== field, 'disabled field remains focusable');
            config = { ...config, multiline: true, editable: false };
            input(); const old = field; field = wrapper._field;
            check(old !== field && !old.isConnected && field.tagName === 'TEXTAREA', 'field kind was not replaced');
            named('远程主机', '地址已更新');
            check(field.disabled && field.readOnly, 'replacement lost disabled or readonly state');
            props = { ...props, disabled: false }; update();
            check(!field.disabled && !field.hasAttribute('aria-disabled') && field.readOnly, 'enabling a readonly control changed readonly semantics');
            field.focus(); check(document.activeElement === field, 'readonly field should remain focusable for selection and copy');
            props = { focusable: true }; update(); named(null, null);
            check(!field.hasAttribute('role') && !field.hasAttribute('aria-expanded'), 'obsolete field ARIA state survived');
            config = { ...config, multiline: false, editable: true }; input(); field = wrapper._field;
            check(field.tagName === 'INPUT' && !field.disabled && !field.readOnly, 'returning to an editable input retained stale state');
            player.op_al(['al', 2, '主机地址']); named('主机地址', null);
            wrapper.remove();
          }
          const button = document.createElement('div');
          Object.assign(button, { _id: 3, _events: new Set(), _pressable: true });
          document.body.append(button); player.views.set(3, button);
          player.op_vp(['vp', 3, { label: '普通按钮', hint: '按钮提示', disabled: true }]);
          check(button.getAttribute('role') === 'button' && button.getAttribute('aria-label') === '普通按钮'
            && button.getAttribute('aria-description') === '按钮提示' && button.getAttribute('aria-disabled') === 'true', 'non-input semantics changed');
          document.body.dataset.result = 'passed';
        } catch (error) { document.body.dataset.result = 'failed: ' + error.message; }
        </script>`);
      const html = execFileSync(chromium, ['--headless', '--no-sandbox', '--disable-dev-shm-usage',
        '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--virtual-time-budget=1000',
        '--user-data-dir=' + join(dir, 'profile'), '--dump-dom', pathToFileURL(join(dir, 'index.html')).href],
        { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 20000, maxBuffer: 1024 * 1024 });
      assert.equal(html.match(/data-result="([^"]*)"/)?.[1], 'passed');
    } finally { await rm(dir, { recursive: true, force: true }); }
  });
