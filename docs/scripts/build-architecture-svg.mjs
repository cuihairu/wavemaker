// Build the static architecture SVGs from the node data below (plan §2.1).
// Outputs: docs/public/architecture.svg (English) and docs/public/architecture.svg.zh
// Visual language mirrors the fleet house style (Apollo docs theme):
// light canvas #f8fafc, white nodes with colored borders, solid edges =
// business requests / results, dashed edges = registration & management.
import { writeFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const W = 1280
const H = 1250

const COLOR = {
  client: '#64748b',
  control: '#2563eb',
  slate: '#475569',
  green: '#087f5b',
  orange: '#c2410c',
  indigo: '#6366f1',
  purple: '#7c3aed',
}

const NODES = [
  {
    id: 'dashboard', x: 470, y: 110, w: 340, h: 64,
    fill: '#fff', stroke: COLOR.client,
    name: { en: 'Web Dashboard / CLI', zh: 'Web Dashboard / CLI' },
  },
  {
    id: 'controller', x: 380, y: 244, w: 520, h: 128,
    fill: '#fff', stroke: COLOR.control,
    name: { en: 'Wavemaker Controller', zh: 'Wavemaker Controller' },
    body: { en: 'Rust + Axum + Tokio', zh: 'Rust + Axum + Tokio' },
    chips: ['Plan API', 'Backend API', 'Report API'],
  },
  {
    id: 'protocol', x: 340, y: 444, w: 600, h: 72,
    fill: '#eef2ff', stroke: COLOR.indigo,
    name: { en: 'Control Protocol', zh: 'Control Protocol（控制协议）' },
    body: { en: 'Protobuf', zh: 'Protobuf' },
    center: true,
  },
  {
    id: 'worker', x: 110, y: 576, w: 470, h: 300,
    fill: '#eef8f5', stroke: COLOR.green,
    name: { en: 'Native Worker', zh: 'Native Worker' },
    body: { en: 'Rust + Tokio', zh: 'Rust + Tokio' },
    lines: ['Actor Runtime', 'Scheduler', 'Virtual Clock', 'Transport', 'Protocol', 'Metrics'],
  },
  {
    id: 'supervisor', x: 700, y: 576, w: 470, h: 120,
    fill: '#fff7ed', stroke: COLOR.orange,
    name: { en: 'Engine Bot Supervisor', zh: 'Engine Bot Supervisor' },
    body: { en: 'Rust', zh: 'Rust' },
  },
  {
    id: 'botprocess', x: 760, y: 756, w: 350, h: 150,
    fill: '#fff', stroke: COLOR.orange,
    name: { en: 'Engine Bot Process', zh: 'Engine Bot Process' },
    lines: ['KBEngine Bots', 'C++ Runtime', 'Python Bridge'],
  },
  {
    id: 'basin', x: 240, y: 986, w: 800, h: 116,
    fill: '#f5f3ff', stroke: COLOR.purple,
    name: { en: 'Basin', zh: 'Basin' },
    lines: ['Topology · Providers · Metric Mapping · Queries', 'Prometheus / OTel / Custom Provider'],
    center: true,
  },
  {
    id: 'report', x: 440, y: 1142, w: 400, h: 64,
    fill: '#fff', stroke: COLOR.purple,
    name: { en: 'Swell Report Pipeline', zh: 'Swell Report Pipeline（报告管线）' },
    center: true,
  },
]

const node = (id) => NODES.find((n) => n.id === id)

// edges: [from, to, d, color, {dashed, label:{en,zh}, labelAt:[x,y], anchor}]
const EDGES = [
  ['dashboard', 'controller', 'M640 174 V244', COLOR.control, { label: { en: 'HTTP / WebSocket', zh: 'HTTP / WebSocket' }, labelAt: [654, 216] }],
  ['controller', 'protocol', 'M640 372 V444', COLOR.slate, {}],
  ['protocol', 'worker', 'M430 516 V546 H345 V576', COLOR.green, {}],
  ['protocol', 'supervisor', 'M850 516 V546 H935 V576', COLOR.green, {}],
  ['worker', 'controller', 'M160 576 V330 H380', COLOR.orange, { dashed: true, label: { en: 'register / lease', zh: '注册 / 租约' }, labelAt: [174, 470] }],
  ['supervisor', 'controller', 'M1120 576 V330 H900', COLOR.orange, { dashed: true, label: { en: 'register / lease', zh: '注册 / 租约' }, labelAt: [1010, 318], anchor: 'middle' }],
  ['supervisor', 'botprocess', 'M935 696 V756', COLOR.orange, { dashed: true, label: { en: 'Local IPC / RPC', zh: 'Local IPC / RPC' }, labelAt: [949, 732] }],
  ['worker', 'basin', 'M345 876 V986', COLOR.indigo, { label: { en: 'metrics', zh: '指标' }, labelAt: [359, 940] }],
  ['botprocess', 'basin', 'M935 906 V986', COLOR.indigo, { label: { en: 'metrics', zh: '指标' }, labelAt: [949, 952] }],
  ['basin', 'report', 'M640 1102 V1142', COLOR.purple, {}],
]

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

function nodeSvg(n, lang) {
  const t = (o) => esc(o[lang] ?? o.en)
  const cx = n.x + n.w / 2
  let out = `<rect x="${n.x}" y="${n.y}" width="${n.w}" height="${n.h}" rx="10" fill="${n.fill}" stroke="${n.stroke}" stroke-width="2"/>\n`
  let y = n.y + (n.lines || n.chips ? 36 : 40)
  const nameX = n.center || !n.lines ? cx : n.x + 20
  const anchor = n.center || !n.lines ? 'middle' : 'start'
  out += `<text x="${nameX}" y="${y}" class="name" text-anchor="${anchor}">${t(n.name)}</text>\n`
  if (n.body) {
    y += 26
    out += `<text x="${nameX}" y="${y}" class="body" text-anchor="${anchor}">${t(n.body)}</text>\n`
  }
  if (n.chips) {
    const chipY = n.y + n.h - 50
    const cw = 150
    const gap = (n.w - n.chips.length * cw) / (n.chips.length + 1)
    n.chips.forEach((c, i) => {
      const cxp = n.x + gap + i * (cw + gap)
      out += `<rect x="${cxp}" y="${chipY}" width="${cw}" height="34" rx="8" fill="#fff" stroke="${n.stroke}" stroke-width="1.5"/>\n`
      out += `<text x="${cxp + cw / 2}" y="${chipY + 23}" class="small" text-anchor="middle">${esc(c)}</text>\n`
    })
  }
  if (n.lines) {
    let ly = y + (n.body ? 36 : 34)
    for (const line of n.lines) {
      out += `<text x="${n.center ? cx : n.x + 20}" y="${ly}" class="${n.center ? 'body' : 'module'}" text-anchor="${n.center ? 'middle' : 'start'}">${esc(line)}</text>\n`
      ly += 30
    }
  }
  return out
}

function edgeSvg(e, lang) {
  const [, , d, color, opts = {}] = e
  const dash = opts.dashed ? ' class="edge control"' : ' class="edge"'
  const marker = `<path d="${d}"${dash} style="stroke:${color};stroke-width:2" marker-end="url(#arrow-${color.slice(1)})"/>`
  const halo = `<path d="${d}" fill="none" stroke="#f8fafc" stroke-width="9"/>`
  let label = ''
  if (opts.label) {
    const [lx, ly] = opts.labelAt
    const anchor = opts.anchor ?? 'start'
    label = `<text x="${lx}" y="${ly}" class="label" text-anchor="${anchor}">${esc(opts.label[lang] ?? opts.label.en)}</text>\n`
  }
  return halo + '\n' + marker + '\n' + label
}

function render(lang) {
  const titles = {
    en: ['Wavemaker · System Architecture',
      'Control plane, execution backends, and observability · solid = business request / result · dashed = registration & management'],
    zh: ['Wavemaker · 系统架构',
      '控制面、执行后端与观测面 · 实线＝业务请求 / 结果 · 虚线＝注册与管理'],
  }
  const markerColors = [...new Set(EDGES.map((e) => e[3]))]
  const defs = markerColors
    .map((c) => `<marker id="arrow-${c.slice(1)}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10Z" fill="${c}"/></marker>`)
    .join('')
  const body = EDGES.map((e) => edgeSvg(e, lang)).join('\n') + '\n' + NODES.map((n) => nodeSvg(n, lang)).join('\n')
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" role="img" aria-labelledby="title desc" xml:lang="${lang === 'zh' ? 'zh-CN' : 'en-US'}">
<title id="title">${esc(titles[lang][0])}</title>
<desc id="desc">${lang === 'zh'
    ? 'Wavemaker 系统架构：Web Dashboard 与 CLI 经 HTTP/WebSocket 访问 Wavemaker Controller（Rust + Axum + Tokio，提供 Plan / Backend / Report 三个 API）；计划与结果经 Protobuf 控制协议到达 Native Worker 与 Engine Bot Supervisor 两个执行后端；Supervisor 经本地 IPC/RPC 管理 Engine Bot Process（KBEngine Bots、C++ Runtime、Python Bridge）；指标汇入 Basin（Topology / Providers / Metric Mapping / Queries，Prometheus / OTel / Custom）并进入 Swell Report Pipeline。实线为业务请求与结果，虚线为注册与管理。'
    : 'Wavemaker system architecture: the Web Dashboard and CLI reach the Wavemaker Controller (Rust + Axum + Tokio, exposing Plan / Backend / Report APIs) over HTTP/WebSocket; plans and results flow through the Protobuf control protocol to the two execution backends, the Native Worker and the Engine Bot Supervisor; the Supervisor manages Engine Bot processes (KBEngine Bots, C++ Runtime, Python Bridge) over local IPC/RPC; metrics converge in the Basin (Topology / Providers / Metric Mapping / Queries, Prometheus / OTel / Custom) and feed the Swell Report Pipeline. Solid lines are business requests and results; dashed lines are registration and management.'}</desc>
<defs>${defs}</defs>
<style>text{font-family:Arial,'Noto Sans CJK SC','Microsoft YaHei',sans-serif;fill:#172b4d}.title{font-size:30px;font-weight:700}.name{font-size:23px;font-weight:700}.body{font-size:17px;fill:#435570}.small{font-size:15px;fill:#52647b}.module{font-size:17px;fill:#435570}.label{font-size:15px;fill:#435570;paint-order:stroke;stroke:#f8fafc;stroke-width:5px;stroke-linejoin:round}.edge{fill:none;stroke-linejoin:round}.control{stroke-dasharray:7 5}</style>
<rect width="${W}" height="${H}" rx="20" fill="#f8fafc"/>
<text x="40" y="48" class="title" text-anchor="start">${esc(titles[lang][0])}</text>
<text x="40" y="80" class="body" text-anchor="start">${esc(titles[lang][1])}</text>
${body}
</svg>
`
}

const here = dirname(fileURLToPath(import.meta.url))
const outDir = join(here, '..', 'public')
mkdirSync(outDir, { recursive: true })
writeFileSync(join(outDir, 'architecture.svg'), render('en'))
writeFileSync(join(outDir, 'architecture-zh.svg'), render('zh'))
console.log('architecture.svg + architecture-zh.svg written to docs/public/')
