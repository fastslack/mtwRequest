#!/usr/bin/env bash
# =============================================================================
# mtwRequest — Quick App Scaffold
# =============================================================================
#
# Usage:
#   ./scripts/create-app.sh svelte my-dashboard
#   ./scripts/create-app.sh react my-app
#   ./scripts/create-app.sh vue my-portal
#
# This creates a ready-to-run app that connects to mtwRequest on port 7741
# and shows real-time data from channels.

set -e

FRAMEWORK="${1:-svelte}"
APP_NAME="${2:-mtw-app}"
MTW_PORT="${MTW_PORT:-7741}"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

info() { echo -e "${CYAN}[mtw]${NC} $1"; }
ok()   { echo -e "${GREEN}[mtw]${NC} $1"; }
err()  { echo -e "${RED}[mtw]${NC} $1"; exit 1; }

# Check bun
command -v bun >/dev/null 2>&1 || err "bun is required. Install: https://bun.sh"

case "$FRAMEWORK" in
  svelte)
    info "Creating SvelteKit app: $APP_NAME"
    bunx create-svelte@latest "$APP_NAME" --template skeleton --types typescript --no-add-ons
    cd "$APP_NAME"

    info "Installing mtwRequest + shadcn-svelte..."
    bun add @matware/mtw-request-ts-client @matware/mtw-request-svelte
    bun add -d tailwindcss @tailwindcss/vite

    # Create a real-time page
    mkdir -p src/routes
    cat > src/routes/+page.svelte << 'SVELTE_EOF'
<script lang="ts">
  import { mtw, channel } from '@matware/mtw-request-svelte';
  import { onMount } from 'svelte';

  const MTW_URL = 'ws://localhost:7741/ws';

  let connected = $state(false);
  let error = $state('');

  const dashboard = channel('dashboard');
  const notifications = channel('notifications');
  const chat = channel('chat.general');

  let chatInput = $state('');

  onMount(async () => {
    try {
      await mtw.connect({ url: MTW_URL });
      connected = true;
    } catch (e: any) {
      error = e.message || 'Connection failed';
    }
  });

  function sendChat() {
    if (!chatInput.trim()) return;
    chat.publish({ user: 'me', text: chatInput, time: Date.now() });
    chatInput = '';
  }
</script>

<main class="min-h-screen bg-gray-950 text-white p-8">
  <h1 class="text-3xl font-bold mb-2">mtwRequest Dashboard</h1>
  <p class="text-gray-400 mb-8">
    {#if connected}
      <span class="text-green-400">● Connected</span> to {MTW_URL}
    {:else if error}
      <span class="text-red-400">● Error:</span> {error}
    {:else}
      <span class="text-yellow-400">● Connecting...</span>
    {/if}
  </p>

  <div class="grid grid-cols-1 md:grid-cols-3 gap-6">
    <!-- Dashboard Channel -->
    <div class="bg-gray-900 rounded-xl p-6 border border-gray-800">
      <h2 class="text-lg font-semibold mb-4">📊 Dashboard</h2>
      <pre class="text-sm text-gray-300 overflow-auto max-h-48">
        {JSON.stringify($dashboard, null, 2) || 'Waiting for data...'}
      </pre>
    </div>

    <!-- Notifications Channel -->
    <div class="bg-gray-900 rounded-xl p-6 border border-gray-800">
      <h2 class="text-lg font-semibold mb-4">🔔 Notifications</h2>
      <div class="space-y-2 max-h-48 overflow-auto">
        {#each $notifications.messages as msg (msg.id)}
          <div class="text-sm bg-gray-800 rounded p-2">
            {JSON.stringify(msg.payload.kind === 'Json' ? msg.payload.data : msg.payload)}
          </div>
        {/each}
        {#if $notifications.messages.length === 0}
          <p class="text-gray-500 text-sm">No notifications yet</p>
        {/if}
      </div>
    </div>

    <!-- Chat Channel -->
    <div class="bg-gray-900 rounded-xl p-6 border border-gray-800">
      <h2 class="text-lg font-semibold mb-4">💬 Chat</h2>
      <div class="space-y-2 max-h-36 overflow-auto mb-4">
        {#each $chat.messages as msg (msg.id)}
          <div class="text-sm bg-gray-800 rounded p-2">
            {JSON.stringify(msg.payload.kind === 'Json' ? msg.payload.data : msg.payload)}
          </div>
        {/each}
        {#if $chat.messages.length === 0}
          <p class="text-gray-500 text-sm">No messages yet</p>
        {/if}
      </div>
      <form onsubmit={(e) => { e.preventDefault(); sendChat(); }} class="flex gap-2">
        <input
          bind:value={chatInput}
          placeholder="Type a message..."
          class="flex-1 bg-gray-800 rounded px-3 py-2 text-sm border border-gray-700 focus:border-blue-500 outline-none"
        />
        <button type="submit" class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium">
          Send
        </button>
      </form>
    </div>
  </div>

  <p class="text-gray-600 text-xs mt-8">
    mtwRequest server: ws://localhost:7741/ws — Start with: cargo run -p mtw-server
  </p>
</main>
SVELTE_EOF

    ok "SvelteKit app created!"
    ok ""
    ok "  cd $APP_NAME"
    ok "  bun run dev"
    ok ""
    ok "  (Make sure mtw-server is running on port $MTW_PORT)"
    ;;

  react)
    info "Creating React (Vite) app: $APP_NAME"
    bunx create-vite "$APP_NAME" --template react-ts
    cd "$APP_NAME"

    info "Installing mtwRequest..."
    bun install
    bun add @matware/mtw-request-ts-client @matware/mtw-request-react

    cat > src/App.tsx << 'REACT_EOF'
import { MtwProvider, useMtw, useChannel, useAgent } from '@matware/mtw-request-react';
import { useState } from 'react';

function Dashboard() {
  const { connected, state } = useMtw();
  const { messages: dashMsgs, lastMessage: dashData } = useChannel('dashboard');
  const { messages: notifMsgs } = useChannel('notifications');
  const { messages: chatMsgs, publish } = useChannel('chat.general');
  const [chatInput, setChatInput] = useState('');

  const sendChat = () => {
    if (!chatInput.trim()) return;
    publish({ user: 'me', text: chatInput, time: Date.now() });
    setChatInput('');
  };

  return (
    <main style={{ minHeight: '100vh', background: '#0a0a0f', color: 'white', padding: 32 }}>
      <h1 style={{ fontSize: 28, fontWeight: 'bold', marginBottom: 8 }}>mtwRequest Dashboard</h1>
      <p style={{ color: '#888', marginBottom: 32 }}>
        {connected
          ? <span style={{ color: '#4ade80' }}>● Connected</span>
          : <span style={{ color: '#fbbf24' }}>● {state}</span>
        }
      </p>

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 24 }}>
        <Card title="📊 Dashboard">
          <pre style={{ fontSize: 12, color: '#ccc', overflow: 'auto', maxHeight: 192 }}>
            {dashData ? JSON.stringify(dashData.payload, null, 2) : 'Waiting for data...'}
          </pre>
        </Card>

        <Card title="🔔 Notifications">
          {notifMsgs.length === 0 && <p style={{ color: '#666', fontSize: 14 }}>No notifications yet</p>}
          {notifMsgs.map(msg => (
            <div key={msg.id} style={{ fontSize: 12, background: '#1a1a2e', borderRadius: 8, padding: 8, marginBottom: 8 }}>
              {JSON.stringify(msg.payload.kind === 'Json' ? msg.payload.data : msg.payload)}
            </div>
          ))}
        </Card>

        <Card title="💬 Chat">
          <div style={{ maxHeight: 140, overflow: 'auto', marginBottom: 12 }}>
            {chatMsgs.length === 0 && <p style={{ color: '#666', fontSize: 14 }}>No messages yet</p>}
            {chatMsgs.map(msg => (
              <div key={msg.id} style={{ fontSize: 12, background: '#1a1a2e', borderRadius: 8, padding: 8, marginBottom: 8 }}>
                {JSON.stringify(msg.payload.kind === 'Json' ? msg.payload.data : msg.payload)}
              </div>
            ))}
          </div>
          <form onSubmit={(e) => { e.preventDefault(); sendChat(); }} style={{ display: 'flex', gap: 8 }}>
            <input
              value={chatInput}
              onChange={e => setChatInput(e.target.value)}
              placeholder="Type a message..."
              style={{ flex: 1, background: '#1a1a2e', border: '1px solid #333', borderRadius: 8, padding: '8px 12px', color: 'white', fontSize: 14, outline: 'none' }}
            />
            <button type="submit" style={{ background: '#2563eb', borderRadius: 8, padding: '8px 16px', color: 'white', fontSize: 14, fontWeight: 500, border: 'none', cursor: 'pointer' }}>
              Send
            </button>
          </form>
        </Card>
      </div>

      <p style={{ color: '#444', fontSize: 11, marginTop: 32 }}>
        mtwRequest server: ws://localhost:7741/ws — Start with: cargo run -p mtw-server
      </p>
    </main>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ background: '#111127', borderRadius: 12, padding: 24, border: '1px solid #222' }}>
      <h2 style={{ fontSize: 16, fontWeight: 600, marginBottom: 16 }}>{title}</h2>
      {children}
    </div>
  );
}

export default function App() {
  return (
    <MtwProvider url="ws://localhost:7741/ws">
      <Dashboard />
    </MtwProvider>
  );
}
REACT_EOF

    ok "React app created!"
    ok ""
    ok "  cd $APP_NAME"
    ok "  bun run dev"
    ok ""
    ok "  (Make sure mtw-server is running on port $MTW_PORT)"
    ;;

  vue)
    info "Creating Vue (Vite) app: $APP_NAME"
    bunx create-vite "$APP_NAME" --template vue-ts
    cd "$APP_NAME"

    info "Installing mtwRequest..."
    bun install
    bun add @matware/mtw-request-ts-client @matware/mtw-request-vue

    cat > src/App.vue << 'VUE_EOF'
<script setup lang="ts">
import { ref, onMounted } from 'vue';
import { useMtw, useChannel } from '@matware/mtw-request-vue';

const { provide: mtwProvide, state, connected } = useMtw('ws://localhost:7741/ws');
mtwProvide();

const dashboard = useChannel('dashboard');
const notifications = useChannel('notifications');
const chat = useChannel('chat.general');

const chatInput = ref('');

function sendChat() {
  if (!chatInput.value.trim()) return;
  chat.publish({ user: 'me', text: chatInput.value, time: Date.now() });
  chatInput.value = '';
}
</script>

<template>
  <main class="min-h-screen bg-gray-950 text-white p-8">
    <h1 class="text-3xl font-bold mb-2">mtwRequest Dashboard</h1>
    <p class="text-gray-400 mb-8">
      <span v-if="connected" class="text-green-400">● Connected</span>
      <span v-else class="text-yellow-400">● {{ state }}</span>
    </p>

    <div class="grid grid-cols-1 md:grid-cols-3 gap-6">
      <div class="bg-gray-900 rounded-xl p-6 border border-gray-800">
        <h2 class="text-lg font-semibold mb-4">📊 Dashboard</h2>
        <pre class="text-sm text-gray-300 overflow-auto max-h-48">
          {{ dashboard.lastMessage ? JSON.stringify(dashboard.lastMessage.payload, null, 2) : 'Waiting for data...' }}
        </pre>
      </div>

      <div class="bg-gray-900 rounded-xl p-6 border border-gray-800">
        <h2 class="text-lg font-semibold mb-4">🔔 Notifications</h2>
        <div class="space-y-2 max-h-48 overflow-auto">
          <div v-for="msg in notifications.messages" :key="msg.id" class="text-sm bg-gray-800 rounded p-2">
            {{ JSON.stringify(msg.payload.kind === 'Json' ? msg.payload.data : msg.payload) }}
          </div>
          <p v-if="notifications.messages.length === 0" class="text-gray-500 text-sm">No notifications yet</p>
        </div>
      </div>

      <div class="bg-gray-900 rounded-xl p-6 border border-gray-800">
        <h2 class="text-lg font-semibold mb-4">💬 Chat</h2>
        <div class="space-y-2 max-h-36 overflow-auto mb-4">
          <div v-for="msg in chat.messages" :key="msg.id" class="text-sm bg-gray-800 rounded p-2">
            {{ JSON.stringify(msg.payload.kind === 'Json' ? msg.payload.data : msg.payload) }}
          </div>
          <p v-if="chat.messages.length === 0" class="text-gray-500 text-sm">No messages yet</p>
        </div>
        <form @submit.prevent="sendChat" class="flex gap-2">
          <input
            v-model="chatInput"
            placeholder="Type a message..."
            class="flex-1 bg-gray-800 rounded px-3 py-2 text-sm border border-gray-700 focus:border-blue-500 outline-none"
          />
          <button type="submit" class="bg-blue-600 hover:bg-blue-500 px-4 py-2 rounded text-sm font-medium">
            Send
          </button>
        </form>
      </div>
    </div>

    <p class="text-gray-600 text-xs mt-8">
      mtwRequest server: ws://localhost:7741/ws — Start with: cargo run -p mtw-server
    </p>
  </main>
</template>
VUE_EOF

    ok "Vue app created!"
    ok ""
    ok "  cd $APP_NAME"
    ok "  bun run dev"
    ok ""
    ok "  (Make sure mtw-server is running on port $MTW_PORT)"
    ;;

  *)
    err "Unknown framework: $FRAMEWORK"
    echo "Usage: $0 <svelte|react|vue> <app-name>"
    ;;
esac
