<script setup lang="ts">
import { computed } from 'vue'
import { useData, withBase } from 'vitepress'

const { lang } = useData()
const english = computed(() => lang.value.startsWith('en'))
const source = computed(() => withBase(english.value ? '/architecture.svg' : '/architecture-zh.svg'))
const description = computed(() => english.value
  ? 'Wavemaker system architecture: the Web Dashboard and CLI reach the Wavemaker Controller (Rust + Axum + Tokio, exposing Plan / Backend / Report APIs) over HTTP/WebSocket; plans and results flow through the Protobuf control protocol to the two execution backends, the Native Worker and the Engine Bot Supervisor; the Supervisor manages Engine Bot processes (KBEngine Bots, C++ Runtime, Python Bridge) over local IPC/RPC; metrics converge in the Basin (Topology / Providers / Metric Mapping / Queries, Prometheus / OTel / Custom) and feed the Swell Report Pipeline. Solid lines are business requests and results; dashed lines are registration and management.'
  : 'Wavemaker 系统架构：Web Dashboard 与 CLI 经 HTTP/WebSocket 访问 Wavemaker Controller（Rust + Axum + Tokio，提供 Plan / Backend / Report 三个 API）；计划与结果经 Protobuf 控制协议到达 Native Worker 与 Engine Bot Supervisor 两个执行后端；Supervisor 经本地 IPC/RPC 管理 Engine Bot Process（KBEngine Bots、C++ Runtime、Python Bridge）；指标汇入 Basin（Topology / Providers / Metric Mapping / Queries，Prometheus / OTel / Custom）并进入 Swell Report Pipeline。实线为业务请求与结果，虚线为注册与管理。')
</script>

<template>
  <figure class="architecture-figure" :aria-label="description">
    <div class="architecture-canvas">
      <img :src="source" width="1280" height="1250" :alt="description" decoding="async" />
    </div>
    <figcaption class="architecture-caption">
      <span>{{ english
        ? 'Target architecture from the design plan. Solid lines are business requests and results; dashed lines are registration and management.'
        : '总体设计目标架构。实线为业务请求与结果，虚线为注册与管理。' }}</span>
      <a :href="source" target="_blank" rel="noopener noreferrer">
        {{ english ? 'Open full-size diagram' : '查看矢量原图' }}
        <svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M7 17 17 7M7 7h10v10" /></svg>
      </a>
    </figcaption>
  </figure>
</template>

<style scoped>
.architecture-figure {
  margin: 28px 0 36px;
  border: 1px solid var(--vp-c-divider);
  border-radius: 12px;
  overflow: hidden;
}
.architecture-canvas {
  background: #fff;
  padding: 12px;
}
.architecture-canvas img {
  display: block;
  width: 100%;
  height: auto;
  margin: 0;
}
.architecture-caption {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px 20px;
  padding: 13px 18px;
  border-top: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-soft);
  color: var(--vp-c-text-2);
  font-size: 13px;
  line-height: 1.6;
}
.architecture-caption a {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  color: var(--vp-c-brand-1);
  font-weight: 500;
  text-decoration: underline;
  text-underline-offset: 3px;
}
.architecture-caption a:focus-visible {
  outline: 2px solid var(--vp-c-brand-1);
  outline-offset: 4px;
  border-radius: 2px;
}
</style>
