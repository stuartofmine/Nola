// SPDX-License-Identifier: Apache-2.0
// Host settings review. Synthetic receipts only; no network, storage or clipboard access.
window.hostConnectionPreview = (() => {
    const example = 'http://studio-mac.local:5001';
    const token = 'demo-pairing-token';
    const devices = {mac:'Mac', ios:'iPhone / iPad', android:'Android'};
    const manages = {mac:'管理消息渠道', ios:'在宿主机上打开插件', android:'管理宿主机模型'};
    const managementTitles = {mac:'消息渠道', ios:'插件', android:'模型'};
    let timer;
    const supports = id => id === 'host-pair' || id === 'share-host';

    function setup(local, state) {
        if (!local.host) local.host = {address:'', token:'', savedAddress:'', permission:'谨慎', used:false};
        const draft = local.host;
        // Direct links to a state also need representative data on first visit.
        if (!draft.address && !['0','manual'].includes(state.id)) {
            draft.address = example;
            draft.token = state.id === 'rejected' ? 'invalid-demo' : token;
        }
        if (state.id === '2' || state.id === 'manage') draft.savedAddress = draft.address;
        return draft;
    }

    function form(draft, state, platform, esc) {
        const readOnly = state.id === 'checking' ? ' disabled' : '';
        const saved = draft.address === draft.savedAddress && !!draft.savedAddress;
        return `<div class="hp-fields">
            <label>宿主机地址<input data-host-field="address" type="url" placeholder="${example}" autocomplete="off" value="${esc(draft.address)}"${readOnly}></label>
            <label>访问令牌<input data-host-field="token" type="password" autocomplete="off" placeholder="${saved?'留空以使用已保存的令牌':'填写这台宿主机的令牌'}" value="${esc(draft.token)}"${readOnly}></label>
            ${platform === 'android' ? '<label data-host-pin '+(/^https:/i.test(draft.address)?'':'hidden')+'>证书 SHA-256（可选）<input data-host-field="pin" autocomplete="off" value="'+esc(draft.pin || '')+'"'+readOnly+'></label>' : ''}
        </div>`;
    }

    function render(scene, state, platform, local, esc) {
        const draft = setup(local, state);
        const share = scene.id === 'share-host';
        const id = state.id;
        const button = (action, index, label = action.label) =>
            `<button type="button" data-transition="${index}" class="${action.style === 'secondary' ? '' : action.style}">${esc(label)}</button>`;
        const removeTitle = {mac:'移除连接',ios:'忘记配对',android:'断开宿主机'};
        const actions = [];
        let toggle = '';
        let manage = '';
        state.actions.forEach((action, index) => {
            if ((action.requires_pairing && !draft.savedAddress) || action.preview_only || (action.platforms && !action.platforms.includes(platform))) return;
            if (share && ['开启共享','关闭共享'].includes(action.label)) {
                const on = action.label === '关闭共享';
                toggle = `<div class="hp-fact"><strong>共享这台 Mac</strong><button type="button" role="switch" aria-checked="${on}" aria-label="${action.label}" data-transition="${index}" class="hp-switch"><span></span></button></div>`;
            } else if (action.to === 'manage') {
                manage = button(action, index, manages[platform]);
            } else {
                let label = action.label;
                if (!share && id === '2' && action.to === '0') label = removeTitle[platform];
                if (!share && id === '2' && action.to === 'checking') label = '重新验证';
                actions.push(button(action, index, label));
            }
        });

        let body = '';
        let status = state.body[0];
        const ready = share && (id === '1' || id === 'copied');
        if (share) {
            const editable = ['0','2','failed'].includes(id);
            body = toggle + (editable
                ? `<label class="hp-mode">权限模式<select data-host-mode><option${draft.permission==='谨慎'?' selected':''}>谨慎</option><option${draft.permission==='全部允许'?' selected':''}>全部允许</option></select></label>`
                : `<p class="hp-fact">权限模式 <strong>${esc(draft.permission)}</strong></p>`);
            if (ready) body += `<p class="hp-fact">共享地址 <strong>${example}</strong></p>`;
            if (id === '0' || id === '2') status = '服务已停止';
            if (ready) status = '服务已就绪';
        } else if (id === '2' || id === 'manage') {
            body = `<p class="hp-fact">宿主机 <strong>${esc(draft.savedAddress)}</strong></p>`;
            if (id === 'manage') body += `<section class="hp-management"><strong>${managementTitles[platform]}</strong><p>${({mac:'尚未连接消息渠道',ios:'尚未安装插件',android:'书房模型 · Responses'})[platform]}</p></section>`;
            if (id === '2' && platform === 'android') body += `<div class="hp-fact"><span>Nola Web Host</span><button data-host-use type="button"${draft.used?' disabled':''}>${draft.used?'已选用':'使用'}</button></div>`;
            status = id === '2' ? '已验证' : '';
        } else if (id !== '0') {
            body = form(draft, state, platform, esc);
        }
        const title = share ? '连接' : id === 'manage' ? '宿主机管理' : '连接宿主机';
        const path = share ? 'Mac 设置' : scene.platform_notes[platform].split('，')[0];
        const mobileSetup = !share && id === '0' && platform !== 'mac';
        const setupHelp = mobileSetup
            ? '<details class="hp-setup-help"><summary>连接前准备</summary><p>在 Mac 的 Nola 设置中打开「连接」，开启共享，再复制连接信息。</p></details>'
            : '';
        let next = '';
        if (ready) next = '<nav class="hp-next" aria-label="继续在客户端演示"><span>在客户端继续预览</span><a href="?scene=host-pair&amp;platform=ios">iPhone / iPad</a><a href="?scene=host-pair&amp;platform=android">Android</a><a href="?scene=host-pair&amp;platform=mac">Mac</a></nav>';
        if (manage) next = `<section class="hp-next"><span>${({mac:'消息渠道',ios:'从设置中的插件库进入',android:'模型管理'})[platform]}</span>${manage}</section>`;
        if (ready) actions.push('<details class="hp-copy-menu"><summary>分别复制</summary><button data-host-copy="地址">复制地址</button><button data-host-copy="令牌">复制访问令牌</button></details>');
        return `<div class="device-system"><span>${devices[platform]}</span><span>交互原型</span></div>
            <section class="hp-panel"><header><p class="hp-path">${esc(path)}</p><h3>${title}</h3></header>
            ${body}${mobileSetup ? '' : `<p role="status" class="hp-note ${state.kind==='error'?'inline-error':''}">${esc(status)}</p>`}
            <p id="field-error" role="alert" class="inline-error"></p><div class="actions">${actions.join('')}</div>${setupHelp}
            ${ready?`<p class="hp-copy-status" role="status">${id==='copied'?'已复制连接信息':''}</p>`:''}${next}</section>`;
    }

    function prepare(scene, current, action, local) {
        if (scene.id !== 'host-pair') return true;
        const draft = local.host;
        if (action.to === 'draft' && action.label === '粘贴连接信息') {
            Object.assign(draft, {address:example, token, pin:''});
        }
        if (action.to === 'checking') {
            const report = (field, error) => {
                document.querySelector('#field-error').textContent = error;
                document.querySelector(`[data-host-field="${field}"]`)?.focus();
                return false;
            };
            let url;
            try { url = new URL(draft.address); } catch { return report('address','请填写有效的宿主机地址。'); }
            if (!['http:','https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash)
                return report('address','请检查地址格式，凭据应填在令牌栏。');
            // Only the in-memory studio fixture has an emulated HTTP listener.
            if (url.protocol === 'http:' && url.hostname !== 'studio-mac.local')
                return report('address','此原型的 HTTP 样例使用 studio-mac.local。公网地址需要 HTTPS。');
            if (draft.pin && !/^[a-f\d]{64}$/i.test(draft.pin.replace(/:/g,'')))
                return report('pin','请填写完整的 SHA-256 证书指纹，或留空。');
            if (!draft.token.trim() && draft.savedAddress !== draft.address)
                return report('token','请填写这台宿主机的访问令牌。');
        }
        if (action.requires_pairing) {
            Object.assign(draft, {address:draft.savedAddress, token:'', pin:''});
        }
        if (current === '2' && action.to === '0') {
            Object.assign(draft, {savedAddress:'', address:'', token:'', pin:'', used:false});
        }
        if (current === 'checking' && action.to === '2') {
            if (draft.address !== draft.savedAddress) draft.used = false;
            Object.assign(draft,{savedAddress:draft.address,token:''});
        }
        return true;
    }

    function schedule(scene, id, local, outcome, advance) {
        if (scene.id === 'share-host' && id === 'starting') {
            timer = setTimeout(() => advance(outcome === 'failed' ? 'failed' : '1'), 1100);
        }
        if (scene.id === 'host-pair' && id === 'checking') {
            const wrongToken = local.host.token && local.host.token !== token;
            let result = outcome;
            if (result === 'ok') {
                if (new URL(local.host.address).hostname !== 'studio-mac.local') result = 'offline';
                else result = wrongToken ? 'rejected' : '2';
            }
            timer = setTimeout(() => advance(result), 1100);
        }
    }

    function edit(field, value, local) {
        const draft = local.host;
        if (field === 'address' && draft.address !== value) {
            draft.token = '';
            draft.pin = '';
            const tokenField = document.querySelector('[data-host-field="token"]');
            if (tokenField) {
                tokenField.value = '';
                tokenField.placeholder = value === draft.savedAddress ? '留空以使用已保存的令牌' : '填写这台宿主机的令牌';
            }
            const pinLabel = document.querySelector('[data-host-pin]');
            if (pinLabel) {
                pinLabel.hidden = !/^https:/i.test(value);
                pinLabel.querySelector('input').value = '';
            }
        }
        draft[field] = value;
        document.querySelector('#field-error').textContent = '';
    }
    return {supports, render, prepare, schedule, edit, cancel:() => clearTimeout(timer)};
})();
