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

- **Frontend** — rota `packages/local-web/src/routes/` → container `packages/web-core/src/{features,pages,shared}/` → primitivo `packages/ui/src/components/` (`KanbanBoard.tsx`, `cn()`). Board em `features/kanban/ui/WorkspacesBoardContainer.tsx`; HTTP em `shared/lib/api.ts`; config e idioma em `i18n/` + `ConfigProvider.tsx`.
- **Backend** — `crates/server/src/routes/` (`workspaces/{create,git,pr,attachments,integration}.rs`, `sessions/mod.rs`) → `crates/services/src/services/` (watcher, pr_monitor, notificações), `crates/db/` (models SQLx + migrations com checksum de bytes), `crates/git` (`GitService`), `crates/{workspace,worktree}-manager`.
- **Agentes** — `crates/executors/src/`: `executors/mod.rs` (spawn), `command.rs`, `profile.rs`, `model_selector.rs`, `mcp_config.rs`; servidor MCP em `crates/mcp/src/task_server/`.
- **Contratos e ambiente** — `crates/api-types/` → `shared/types.ts` (gerado por ts-rs, não editar à mão); dev server e serviço em `.claude/skills/run-vibe-kanban/`.

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

Dentro dessa cadeia, `discover_options` já não é uma foto estática — cada executor decide
por si o quão dinâmico consegue ser. Codex spawna `codex app-server` e chama `model/list`
por JSON-RPC (metadado local, zero token). Claude Code é híbrido: tabela estática de
apelidos com display "apelido → Nome Real" (`fable`/`fable[1m]`/`opus`/`opus[1m]`/`sonnet`/
`haiku`, pin `2.1.223`) por cima de um overlay lido de `~/.claude.json`
(`additionalModelOptionsCache`/`orgModelDefaultCache`), deduplicado por `CANONICAL_MODEL_IDS`
(claude.rs). Gemini continua 100% estático — o ACP `session/new` só devolve
`availableModels` a partir da CLI 0.36.0, e está bloqueado nesta conta (gemini.rs). O
esforço de raciocínio segue o mesmo espírito: `ReasoningOption::from_names_with_default`
tira o "high" cravado do default, cada modelo com sua própria matriz (`haiku` fica sem
`xhigh`/`max`, os demais vão `low`→`max`), e o default do usuário vem de `default_reasoning_effort`
(Config v8, `GeneralSettingsSection`) resolvido em `modelSelector.ts`/
`ModelSelectorContainer.tsx` na ordem override → preset → recente → configurado →
`is_default` do modelo.

**Executors** — `ExecutorError` (159 arestas) é o eixo; `command.rs` monta comandos
(`CommandBuilder`, `CmdOverrides` por agente: Codex/ClaudeCode/Opencode/CursorAgent/Copilot),
`profile.rs` resolve perfis (`ExecutorProfileId`, `ExecutorConfigs`), `mod.rs` faz spawn
(`SpawnedChild`), `logs/` normaliza saída por agente, `mcp_config.rs` adapta MCP por agente.
Toda lista estática de modelos carrega `// zona de envelhecimento — revisar no merge
mensal` (fallback do Codex e catálogo do Gemini) — o pin da CLI e o catálogo andam juntos,
e é o merge mensal que revisa os dois.

**Regra do frescor** — boot dispara `cli_freshness::refresh_if_stale` fire-and-forget
(`local-deployment/src/lib.rs`), que compara os pins `npx` dos executores com
`registry.npmjs.org/{pkg}/latest` e cacheia por 24h em `cli_freshness.json`
(`crates/services/src/services/cli_freshness.rs`). Zero LLM, zero token. `GET
/cli-freshness` serve o cache; a `AppBar` mostra aviso discreto via `useCliFreshness` +
`SharedAppLayout`. Quando acusar desatualizado, o `pin_hint` de cada entrada aponta o
arquivo/linha do `base_command` a bumpar — mesma cadência do merge mensal acima.

**Rotas workspaces/sessions** — todas passam por `ApiResponse` + `Deployment`:
`workspaces/git.rs` (deg 43, o mais pesado, com `git_ops_safety.rs`), `create.rs`
(`create_and_start_workspace`), `pr.rs`, `attachments.rs`, `integration.rs` (abrir editor),
`sessions/mod.rs` (`follow_up`).

**i18n** — comunidade 1: `i18n/config.ts` + `languages.ts` + `index.ts`, plugado em
`ConfigProvider` e `GeneralSettingsSection`. Locales: `en, es, fr, ja, ko, zh-Hans,
zh-Hant, pt-BR`. Registrar idioma são **quatro** pontos, não três: o dropdown nasce do
enum Rust `UiLanguage` (`config/versions/v6.rs` + `pnpm generate-types`), além de
`SUPPORTED_I18N_CODES`, `getEndonym()` e a pasta em `locales/`. **Regra da casa:**
string nova entra manualmente em `en/*.json` (não há extração automática); locale
desatualizado cai em fallback inglês (`fallbackLng: en`) e a bateria acusa nos dois
sentidos — chave morta via `check-unused-i18n-keys.mjs` (`pnpm lint`), hardcode via
`i18next/no-literal-string` (`check-i18n.sh`). Namespaces 100% sunset (`organization`,
`projects`) ficam em inglês de propósito.

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

Os envelopes: `Result` (web-core `lib/api.ts`, 1106 arestas) em toda chamada HTTP do front
e `ApiResponse` (crates/utils, 187) em toda rota axum. Os eixos: `ExecutorError` (159) nos
agentes, `Error` (executors, 277) no log normalizado, `GitService` (91) e `MsgStore` (84).
`cn()` aparece duas vezes — 219 em `packages/ui` e 85 num clone em web-core: duplicação
real, não artefato do grafo.

## Perguntas que este grafo responde bem

- Como o board deriva colunas de `WorkspaceWithStatus`? (c134 → `useWorkspaces` → `workspacesApi`)
- Onde um modelo novo precisa ser registrado nas 3 camadas? (`model_selector.rs` → `modelSelector.ts` → popover)
- Qual rota axum toca git e por que é a mais arriscada? (`workspaces/git.rs` + `git_ops_safety.rs`, e há ciclo `deployment ⇄ git.rs`)
- Que arquivos vivos ainda dependem da zona do sunset? (vizinhos de `sunset_zone`)
- Como o dev server escolhe porta e por que ela muda? (hyperedge do startup, `.dev-ports.json`)

## Honestidade sobre este mapa

- **Fora do escopo, de propósito:** 71 `.mdx` de docs/marketing, 328 imagens, 8 `.wav`,
  `npx-cli/`, `scripts/`, `shared/` gerado, `.github/`.
- **Não use contagem de arestas como métrica de acoplamento:** 4.033 arestas têm ponta
  solta (símbolos externos como `chrono`/`react`, sem nó), ~500 pares vêm colapsados
  (`calls` + `references` no mesmo par), 386 `query-*.json` (cache SQLx) não geraram nós e
  1.531 nós são isolados (tipos gerados). Extração: 99% EXTRACTED por AST, 1% INFERRED
  (303 arestas, confiança média 0.76), 2 AMBIGUOUS — ~151k tokens, só nos 12 docs.
