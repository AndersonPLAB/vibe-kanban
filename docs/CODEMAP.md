# Mapa de bairro — vibe-kanban (fork local-first)

Documento de **orientação**, não inventário. Leia isto antes de abrir arquivos; o
inventário completo é o grafo (`graphify-out/`), não este texto.

Gerado por graphify em 2026-08-06 sobre `crates/` + `packages/` + 12 docs de
arquitetura, no commit `a7fcf06f`. **11.316 nós · 28.318 arestas · 402 comunidades.**
Regenerar: `/graphify` (escopo: crates + packages, sem imagens/.mdx/.wav).

## Como navegar

1. **Pergunte ao grafo primeiro:** `graphify query "como o board carrega workspaces?"`,
   `graphify path "WorkspacesBoardContainer.tsx" "GitService"`, `graphify explain "MsgStore"`.
   Grep depois, para editar linhas específicas.
2. **O fluxo é sempre o mesmo:** rota (`packages/local-web/src/routes/*.tsx`) →
   container (`packages/web-core/src/**`) → primitivo (`packages/ui/src/components/*`)
   ⇄ `api.ts` → rota axum (`crates/server/src/routes/**`) → service/db.
3. **Não entre na zona do sunset** (abaixo). Código lá compila e não roda.

## Bairros

| Bairro | Onde | Porta de entrada |
|---|---|---|
| Board (nosso) | `packages/web-core/src/features/kanban/ui/` | `WorkspacesBoardContainer.tsx` |
| Rotas do app | `packages/local-web/src/routes/` | `_app.workspaces_.board.tsx`, `routeTree.gen.ts` |
| Biblioteca de UI | `packages/ui/src/components/` | `KanbanBoard.tsx`, `Button.tsx`, `cn()` |
| Shell compartilhado | `packages/web-core/src/shared/` | `lib/api.ts`, `components/ui-new/containers/` |
| Config + i18n | `packages/web-core/src/i18n/`, `ConfigProvider.tsx` | `config.ts`, `languages.ts` |
| API HTTP | `crates/server/src/routes/` | `workspaces/{create,git,pr,attachments,integration}.rs`, `sessions/mod.rs` |
| Agentes de código | `crates/executors/src/` | `executors/mod.rs`, `command.rs`, `profile.rs`, `model_selector.rs` |
| Git | `crates/git/src/lib.rs` | `GitService` |
| Worktrees/workspaces | `crates/workspace-manager/`, `crates/worktree-manager/` | `worktree_manager.rs` |
| Persistência | `crates/db/src/models/`, `crates/db/migrations/` | `models/*.rs` (SQLx, checksum de bytes) |
| Serviços de fundo | `crates/services/src/services/` | `filesystem_watcher.rs`, `pr_monitor`, notificações |
| Tipos compartilhados | `crates/api-types/`, `shared/types.ts` | gerados por ts-rs — não editar `shared/` |
| MCP | `crates/mcp/src/task_server/` | adapters em `executors/src/mcp_config.rs` |
| Dev/serviço | `.claude/skills/run-vibe-kanban/` | `SKILL.md`, `driver.mjs`, `open.mjs` |

## Nossos pontos de interesse

**Board `/workspaces/board` (Variante A)** — comunidade 134, o subgrafo mais coeso do fork:

```
_app.workspaces_.board.tsx  (rota)
  → WorkspacesBoardContainer.tsx   deg 28  ← o cérebro: colunas derivadas
      → useWorkspaces() · useRepoBranches() · useAppNavigation() · workspacesApi
      → KanbanBoard.tsx (packages/ui)  deg 22 → KanbanBoard/KanbanCards/KanbanCard + cn()
```

Colunas (Queued/Running/Errored/Done/Archived) são **derivadas** de
`WorkspaceWithStatus` — não há coluna persistida. Um workspace sem repo é
estruturalmente inválido (500 `Workspace has no repositories configured`).

**Skill run-vibe-kanban / serviço + ícone** — comunidades 270 e 296, ligadas por um
hyperedge real ("Windows dev-server startup and port-discovery flow"):
`open.mjs` ⇄ `.dev-ports.json` ⇄ `scripts/setup-dev-environment.js`, com
`driver.mjs` no centro (varredura de órfãos, libclang, bash+coreutils, trap de CRLF).
Segundo hyperedge: "Three undocumented Windows preflight blockers".

**Seleção de modelo** — atravessa três camadas, e a fonte da verdade é o Rust:

```
crates/executors/src/model_selector.rs   ModelSelectorConfig · ModelInfo · ReasoningOption
        ↓ (ts-rs → shared/types.ts)
packages/web-core/src/shared/lib/modelSelector.ts   parseModelId · resolveDefaultModelId
        ↓
ModelSelectorContainer.tsx  deg 39  → ModelSelectorPopover.tsx (packages/ui)
        ↑ consumidores: SessionChatBoxContainer · CreateChatBoxContainer
```

**Executors** — `ExecutorError` (159 arestas) é o eixo; `command.rs` monta comandos
(`CommandBuilder`, `CmdOverrides` por agente: Codex/ClaudeCode/Opencode/CursorAgent/Copilot),
`profile.rs` resolve perfis (`ExecutorProfileId`, `ExecutorConfigs`), `mod.rs` faz spawn
(`SpawnedChild`), `logs/` normaliza saída por agente, `mcp_config.rs` adapta MCP por agente.

**Rotas workspaces/sessions** — todas passam por `ApiResponse` + `Deployment`:
`workspaces/git.rs` (deg 43, o mais pesado, com `git_ops_safety.rs`), `create.rs`
(`create_and_start_workspace`), `pr.rs`, `attachments.rs`, `integration.rs` (abrir editor),
`sessions/mod.rs` (`follow_up`).

**i18n** — comunidade 1: `i18n/config.ts` + `languages.ts` + `index.ts`, plugado em
`ConfigProvider` e `GeneralSettingsSection`. Locales presentes nesta branch:
`en, es, fr, ja, ko, zh-Hans, zh-Hant`. **Não há `pt-BR` aqui** — se existe, está em
outra branch/worktree; adicionar significa mexer em `SUPPORTED_I18N_CODES`,
`getEndonym()` e uma pasta nova em `i18n/locales/`.

## Zona do sunset — não usar

Um único nó no grafo (`sunset_zone`), representando **1.989 símbolos de 493 arquivos**
da era cloud, colapsados de propósito para não poluir o mapa:

- `crates/remote`, `crates/remote-info`, `crates/relay-*` (9 crates)
- `packages/remote-web` inteiro
- `web-core`: `integrations/remote/`, `shared/integrations/electric/`,
  `pages/kanban/ProjectKanban.tsx`, `pages/workspaces/ElectricTestPage.tsx`
- rotas cloud-only em `local-web`: qualquer `issues.$issueId`, `hosts.$hostId`, `electric-test`

Inerte no build local (`VK_SHARED_API_BASE` ausente ⇒ "remote features disabled",
"relay unavailable"). **Não estender, não copiar padrão de lá.**

A fronteira não está limpa, e ela é menor e mais funda do que parece — ver
"Fronteira do Sunset" abaixo.

Cinza, não colapsado: `crates/services/src/services/remote_client.rs`
(`RemoteClient`, 92 arestas; `RemoteClientError`, 80) — vive em `services/`, é usado
condicionalmente e fica inerte sem cloud. Trate como sunset-adjacente.

## Fronteira do Sunset

Direção das arestas, não vizinhança: **44 arquivos vivos dependem** do sunset; **121**
são apenas importados por ele (direção inofensiva); **969** "vizinhos" eram símbolos
externos, não arquivos. _(Correção: uma versão anterior deste mapa citava 206/80/30 —
aquilo somava as duas direções mais símbolos externos.)_

| Categoria | N | O que é |
|---|---|---|
| REAL — TS/Electric | 12 | importam `useShape` de `shared/integrations/electric/hooks` (runtime, live query). Montam, tentam sincronizar, falham em silêncio sem cloud |
| REAL — plumbing de relay em Rust | 12 | `server/src/relay_pairing/`, `routes/relay_auth/`, `routes/host_relay/`, `routes/webrtc.rs`, `runtime/relay_registration.rs`, `middleware/{relay_request_signature,signed_ws}.rs`, `crates/embedded-ssh/*`, `crates/desktop-bridge/*` — cloud dentro de crates "vivos", fora do regex do colapso |
| ESTRUTURAL | 3 | `crates/deployment/src/lib.rs`, `crates/local-deployment/src/lib.rs`, `crates/server/src/error.rs` |
| TIPO | 2 | só `shared/remote-types` (tipos/consts gerados de `crates/remote`), sem execução |
| MORTA-DISFARÇADA | 1 | `LocalProjectKanban.tsx` |

Os 14 restantes são ruído: 12 arestas `indirect_call` INFERRED contra o bundle minificado
`preview-proxy/src/bippy_bundle.js` e 2 colisões de nome (`git2::Remote` vs `crates/remote*`
— inclusive o "campeão" de acoplamento, `crates/git/src/lib.rs`, que não tem nada de cloud).

**Os 5 mais acoplados de verdade** (arestas → sunset, ruído já descontado):

| # | Arquivo | Veredito |
|---|---|---|
| 7 | `crates/local-deployment/src/lib.rs` | ESTRUTURAL — `use relay_control, relay_hosts, relay_webrtc, remote_info, RemoteClient` |
| 7 | `crates/server/src/relay_pairing/server.rs` | REAL — handshake de pareamento de relay |
| 5 | `crates/server/src/routes/relay_auth/server.rs` | REAL |
| 4 | `crates/desktop-bridge/src/ssh_config.rs` | REAL — túnel SSH do desktop bridge |
| 3 | `crates/server/src/error.rs` | ESTRUTURAL — mapeia 6+ erros de relay/remote em `ApiError` |

**O achado que importa:** o cloud não está atrás de um feature flag isolável. `deployment`
(a camada de trait/DI em que o servidor local inteiro se apoia) carrega campos de
`relay_control`/`relay_hosts`/`relay_webrtc`/`remote_info`, e `ApiError` conhece os erros
de relay. Arrancar o sunset é cirurgia nessas 3 costuras, não `rm -rf`.

**MORTA-DISFARÇADA, o caso exemplar:** `LocalProjectKanban.tsx` tem 5 linhas e só faz
`return <ProjectKanban />`. Mora fora do sunset e é montado por **rotas vivas** —
`/_app/projects/$projectId` e 4 rotas `issues`/`hosts`. `ProjectKanban` abre com
`useAuth`, `LoginRequiredPrompt`, `useOrganizationProjects` e renderiza
`ProjectSunsetPage`. Ou seja: **a rota de "projects" do app local termina na página de
sunset**. O board da Variante A (`/workspaces/board`) é o caminho vivo.

**Veredito i18n (pt-BR) — não há bloqueio:** `i18n/index.ts` tem duas linhas
(`import './config'; export { default } from './config';`) e **não importa nada do
sunset**. A aresta no grafo era a direção inversa: código do sunset (`ProjectKanban`,
via `useTranslation`) importa o i18n. Adicionar `pt-BR` é local a três pontos —
`i18n/locales/pt-BR/`, `SUPPORTED_I18N_CODES` e `getEndonym()` em `languages.ts` — e
não toca a fronteira. O único efeito colateral é que páginas do sunset ganhariam chaves
faltando em pt-BR, e elas não são alcançadas pelo caminho vivo.

**Isto é cartografia, não faxina.** Nada foi refatorado. Se um dia for: a ordem de menor
risco é (1) rotas `/projects` → sunset, (2) os 12 hooks Electric, (3) as 3 costuras
estruturais — a última exige redesenhar `Deployment` e `ApiError`.

## O que você vai ver em todo lugar

| Nó | Arestas | O que é |
|---|---|---|
| `Result` (web-core `lib/api.ts`) | 1106 | envelope de toda chamada HTTP no front |
| `Error` (executors) | 277 | erro normalizado de agente |
| `cn()` (packages/ui) | 219 (+85 num clone em web-core) | merge de classes Tailwind |
| `ApiResponse` (crates/utils) | 187 | envelope de toda rota axum |
| `ExecutorError` | 159 | eixo do sistema de agentes |
| `GitService` / `MsgStore` | 91 / 84 | git e streaming de logs |

Dois `cn()` distintos (ui e web-core) são duplicação real, não artefato do grafo.

## Perguntas que este grafo responde bem

- Como o board deriva colunas de `WorkspaceWithStatus`? (c134 → `useWorkspaces` → `workspacesApi`)
- Onde um modelo novo precisa ser registrado nas 3 camadas? (`model_selector.rs` → `modelSelector.ts` → popover)
- Qual rota axum toca git e por que é a mais arriscada? (`workspaces/git.rs` + `git_ops_safety.rs`, e há ciclo `deployment ⇄ git.rs`)
- Que arquivos vivos ainda dependem da zona do sunset? (vizinhos de `sunset_zone`)
- Como o dev server escolhe porta e por que ela muda? (hyperedge do startup, `.dev-ports.json`)

## Honestidade sobre este mapa

- **Fora do grafo:** 71 `.mdx` de docs/marketing, 328 imagens, 8 `.wav`, `npx-cli/`,
  `scripts/`, `shared/` gerado, `.github/`. Escolha deliberada de escopo.
- **Saúde do grafo:** 4.033 arestas com ponta solta (quase todas símbolos externos —
  `chrono`, `react` etc. aparecem como alvo sem nó) e ~500 pares colapsados
  (`calls` + `references` entre o mesmo par). 386 arquivos `query-*.json` (cache SQLx)
  não geraram nós. Nada disso invalida a topologia; não confie em contagem de arestas
  como métrica de acoplamento.
- **1.531 nós isolados** (≤1 conexão): tipos gerados e enums pequenos, não descoberta.
- Extração: 99% EXTRACTED (AST determinístico), 1% INFERRED (303 arestas, confiança
  média 0.76), 2 arestas AMBIGUOUS. Custo: ~151k tokens num subagente, só nos 12 docs.
