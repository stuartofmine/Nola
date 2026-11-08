(function () {
    'use strict';

    var originalFetch = window.fetch.bind(window);
    var unsafeMethods = { POST: true, PUT: true, PATCH: true, DELETE: true };

    function cookie(name) {
        var prefix = name + '=';
        var parts = document.cookie ? document.cookie.split(';') : [];
        for (var i = 0; i < parts.length; i += 1) {
            var part = parts[i].trim();
            if (part.indexOf(prefix) === 0) return part.slice(prefix.length);
        }
        return '';
    }

    function sameOrigin(input) {
        try {
            var raw = typeof input === 'string' ? input : input.url;
            return new URL(raw, window.location.href).origin === window.location.origin;
        } catch (_) {
            return false;
        }
    }

    window.fetch = function (input, options) {
        var init = Object.assign({}, options || {});
        var method = String(init.method || (input && input.method) || 'GET').toUpperCase();
        if (sameOrigin(input)) {
            init.credentials = 'same-origin';
            if (unsafeMethods[method]) {
                var headers = new Headers(init.headers || (input && input.headers) || {});
                var csrf = cookie('nola_web_csrf');
                if (csrf) headers.set('X-Nola-CSRF', csrf);
                init.headers = headers;
            }
        }
        return originalFetch(input, init).then(function (response) {
            if (response.status === 401 && window.location.pathname !== '/login') {
                var next = window.location.pathname + window.location.search;
                window.location.assign('/login?next=' + encodeURIComponent(next));
            }
            return response;
        });
    };

    document.addEventListener('DOMContentLoaded', function () {
        var forms = document.querySelectorAll('form[action="/logout"]');
        forms.forEach(function (form) {
            form.addEventListener('submit', function (event) {
                event.preventDefault();
                window.fetch('/api/web/logout', { method: 'POST' })
                    .finally(function () { window.location.assign('/login?logged_out=1'); });
            });
        });
    });
})();
