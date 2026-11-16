import { defineConfig } from 'vitepress'

const BASE = '/wavemaker/'

// DOCS-LANG-001: browser-language detection. Runs before hydration so no
// EN frame flashes for a Chinese reader. Precedence: ?lang=zh|en (one-shot
// override, remembered) > a recorded manual choice > navigator.language.
// Only the two locale HOME pages redirect — deep links are never hijacked,
// and clicking the language dropdown records an explicit preference.
const langDetect = `(function () {
  var KEY = 'wm-docs-lang';
  var HOME = ${JSON.stringify(BASE)};
  var ZH = HOME + 'zh/';
  function same(a, b) { return a === b || a === b + 'index.html'; }
  var href = location.pathname;
  var atHome = same(href, HOME);
  var atZhHome = same(href, ZH);
  document.addEventListener('click', function (ev) {
    var t = ev.target;
    var a = t && t.closest ? t.closest('a[href]') : null;
    if (!a) return;
    var h = a.getAttribute('href') || '';
    var p = h.replace(${JSON.stringify(BASE)}, '');
    var isZh = p === '/zh/' || p === 'zh/' || p === '/zh/index.html' || p === 'zh/index.html';
    var isEn = p === '/' || p === '' || p === '/index.html' || p === 'index.html';
    if (isZh || isEn) {
      try { localStorage.setItem(KEY, isZh ? 'zh' : 'en'); } catch (e) {}
    }
  }, true);
  if (!atHome && !atZhHome) return;
  var explicit = null;
  try { explicit = localStorage.getItem(KEY); } catch (e) {}
  var forced = new URLSearchParams(location.search).get('lang');
  if (forced === 'zh' || forced === 'en') {
    explicit = forced;
    try { localStorage.setItem(KEY, forced); } catch (e) {}
    var rest = location.search.replace(/[?&]lang=[^&]*/, '');
    if (rest.indexOf('&') === 0) rest = '?' + rest.slice(1);
    history.replaceState(null, '', href + rest);
  }
  if (!explicit) {
    var nav = (navigator.languages && navigator.languages[0]) || navigator.language || '';
    explicit = nav.toLowerCase().indexOf('zh') === 0 ? 'zh' : 'en';
  }
  if (atHome && explicit === 'zh') { location.replace(ZH); }
  else if (atZhHome && explicit === 'en') { location.replace(HOME); }
})();`

export default defineConfig({
  base: BASE,
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: BASE + 'favicon.svg' }],
    ['script', {}, langDetect],
  ],

  title: 'Wavemaker',
  description:
    'A cross-engine game server load testing framework with a unified load model, plan data, execution backends, and observability.',

  locales: {
    root: {
      lang: 'en',
      label: 'English',
      themeConfig: {
        nav: [
          { text: 'Docs', link: '/introduction' },
          { text: 'Roadmap', link: '/roadmap' }
        ],
        sidebar: [
          {
            text: 'Overview',
            items: [{ text: 'Introduction', link: '/introduction' }]
          },
          {
            text: 'Design',
            items: [
              { text: 'Architecture', link: '/architecture' },
              { text: 'Tech Stack', link: '/tech-stack' },
              { text: 'Execution Backends', link: '/backends' },
              { text: 'Engine Bot Bridge', link: '/bridge' },
              { text: 'KBEngine Adapter', link: '/kbengine' },
              { text: 'Process Supervisor', link: '/supervisor' }
            ]
          },
          {
            text: 'Load & Execution',
            items: [
              { text: 'Spectrum & Deterministic Plans', link: '/spectrum' },
              { text: 'Native Runtime', link: '/native-runtime' },
              { text: 'Test Plans', link: '/test-plan' }
            ]
          },
          {
            text: 'Observability',
            items: [
              { text: 'Basin (Server Observability)', link: '/basin' },
              { text: 'Generator Observability', link: '/generator-observability' }
            ]
          },
          {
            text: 'Operations',
            items: [
              { text: 'Distributed Management', link: '/distributed' },
              { text: 'Swell Lifecycle & Reports', link: '/swell' },
              { text: 'Configuration', link: '/configuration' },
              { text: 'Security & Isolation', link: '/security' },
              { text: 'Testing & Validation', link: '/testing' }
            ]
          },
          {
            text: 'Specs',
            items: [
              { text: 'Assertion Provider', link: '/spec/assertion-provider' }
            ]
          },
          {
            text: 'Planning',
            items: [
              { text: 'Roadmap', link: '/roadmap' },
              { text: 'Risks', link: '/risks' },
              { text: 'Decisions', link: '/decisions' }
            ]
          }
        ]
      }
    },
    zh: {
      lang: 'zh-CN',
      label: '简体中文',
      link: '/zh/',
      themeConfig: {
        nav: [
          { text: '文档', link: '/zh/introduction' },
          { text: '实施计划', link: '/zh/roadmap' }
        ],
        sidebar: [
          {
            text: '概述',
            items: [{ text: '项目定位', link: '/zh/introduction' }]
          },
          {
            text: '设计',
            items: [
              { text: '总体架构', link: '/zh/architecture' },
              { text: '技术栈决策', link: '/zh/tech-stack' },
              { text: '执行后端模型', link: '/zh/backends' },
              { text: 'Engine Bot Bridge', link: '/zh/bridge' },
              { text: 'KBEngine Adapter', link: '/zh/kbengine' },
              { text: '进程监督与容器化', link: '/zh/supervisor' }
            ]
          },
          {
            text: '负载与执行',
            items: [
              { text: 'Spectrum 与确定性计划', link: '/zh/spectrum' },
              { text: 'Native Runtime', link: '/zh/native-runtime' },
              { text: '测试计划', link: '/zh/test-plan' }
            ]
          },
          {
            text: '观测',
            items: [
              { text: '服务端观测：Basin', link: '/zh/basin' },
              { text: '发生器观测与数据质量', link: '/zh/generator-observability' }
            ]
          },
          {
            text: '运行与治理',
            items: [
              { text: '分布式管理与故障处理', link: '/zh/distributed' },
              { text: 'Swell 生命周期与报告', link: '/zh/swell' },
              { text: '配置设计草案', link: '/zh/configuration' },
              { text: '安全与隔离', link: '/zh/security' },
              { text: '测试与验证策略', link: '/zh/testing' }
            ]
          },
          {
            text: '规格说明',
            items: [
              { text: 'Assertion Provider 业务断言', link: '/zh/spec/assertion-provider' }
            ]
          },
          {
            text: '计划',
            items: [
              { text: '分阶段实施计划', link: '/zh/roadmap' },
              { text: '风险登记', link: '/zh/risks' },
              { text: '最终技术决策摘要', link: '/zh/decisions' }
            ]
          }
        ]
      }
    }
  },

  themeConfig: {
    logo: '/logo.svg',
    socialLinks: [{ icon: 'github', link: 'https://github.com/cuihairu/wavemaker' }]
  }
})
