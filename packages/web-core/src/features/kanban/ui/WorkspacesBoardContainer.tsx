import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '@tanstack/react-query';
import {
  KanbanProvider,
  KanbanBoard,
  KanbanCard,
  KanbanCards,
  KanbanHeader,
} from '@vibe/ui/components/KanbanBoard';
import {
  KanbanCardContent,
  type KanbanPullRequest,
} from '@vibe/ui/components/KanbanCardContent';
import { PlusIcon } from '@phosphor-icons/react';
import {
  useWorkspaces,
  type SidebarWorkspace,
} from '@/shared/hooks/useWorkspaces';
import { useAppNavigation } from '@/shared/hooks/useAppNavigation';
import { workspacesApi, repoApi } from '@/shared/lib/api';
import { CommandBarDialog } from '@/shared/dialogs/command-bar/CommandBarDialog';
import RepoSelector from '@/shared/components/tasks/RepoSelector';
import { useRepoBranches } from '@/shared/hooks/useRepoBranches';

/**
 * Local-first workspaces board.
 *
 * Columns are *derived* from the workspace's real state rather than stored, so
 * there is no board_status column and no drag-to-move: a card sits where its
 * execution state puts it. Data comes from the same local SQLite stream the
 * sidebar uses (`GET /api/workspaces/streams/ws`), so column membership updates
 * live with no refetch after an action.
 */
type ColumnId = 'queued' | 'running' | 'errored' | 'done' | 'archived';

const COLUMN_DOT_CLASS: Record<ColumnId, string> = {
  queued: 'bg-low',
  running: 'bg-brand',
  errored: 'bg-error',
  done: 'bg-success',
  archived: 'bg-low',
};

/**
 * Precedence matters: a workspace running its setup script is still
 * `hasNeverRun` (no coding agent yet) but belongs in Running, and a re-run after
 * a failure belongs in Running rather than Errored.
 */
function columnFor(workspace: SidebarWorkspace): ColumnId {
  if (workspace.isArchived) return 'archived';
  if (workspace.isRunning) return 'running';
  if (workspace.isErrored) return 'errored';
  if (workspace.hasNeverRun) return 'queued';
  return 'done';
}

/**
 * PrBadge only renders the three resolved states, so a workspace whose PR status
 * has not been resolved yet gets no badge rather than an "unknown" one.
 */
function prBadgesFor(workspace: SidebarWorkspace): KanbanPullRequest[] {
  const { prNumber, prUrl, prStatus } = workspace;
  if (!prNumber || !prUrl || !prStatus || prStatus === 'unknown') {
    return [];
  }
  return [
    { id: String(prNumber), number: prNumber, url: prUrl, status: prStatus },
  ];
}

export function WorkspacesBoardContainer() {
  const { t } = useTranslation('common');
  const appNavigation = useAppNavigation();
  const { workspaces, archivedWorkspaces, isLoading } = useWorkspaces();
  const [isEnqueuing, setIsEnqueuing] = useState(false);

  // A workspace needs at least one repository to have a worktree, so queueing
  // one requires picking the repo up front — there is no sane default to infer
  // locally, and a repo-less workspace breaks every downstream consumer.
  const { data: repos = [] } = useQuery({
    queryKey: ['repos'],
    queryFn: () => repoApi.list(),
  });
  const [selectedRepoId, setSelectedRepoId] = useState<string | null>(null);
  useEffect(() => {
    if (!selectedRepoId && repos.length > 0) {
      setSelectedRepoId(repos[0].id);
    }
  }, [repos, selectedRepoId]);

  // add_repository validates the branch exists, so an empty target_branch is
  // rejected ("Branch '' does not exist"). Resolve a real branch: the repo's
  // configured default, else whatever it currently has checked out.
  const { data: branches = [] } = useRepoBranches(selectedRepoId);
  const targetBranch = useMemo(() => {
    const repo = repos.find((candidate) => candidate.id === selectedRepoId);
    return (
      repo?.default_target_branch ??
      branches.find((branch) => branch.is_current)?.name ??
      null
    );
  }, [repos, selectedRepoId, branches]);

  const columns = useMemo(() => {
    const grouped: Record<ColumnId, SidebarWorkspace[]> = {
      queued: [],
      running: [],
      errored: [],
      done: [],
      archived: archivedWorkspaces,
    };
    for (const workspace of workspaces) {
      grouped[columnFor(workspace)].push(workspace);
    }
    return grouped;
  }, [workspaces, archivedWorkspaces]);

  // A queued workspace has no prompt stored, so there is nothing to dispatch
  // from here. Opening it lands on its new-session prompt box, which is where
  // the agent actually gets started.
  const handleOpenWorkspace = useCallback(
    (workspaceId: string) => {
      appNavigation.goToWorkspace(workspaceId);
    },
    [appNavigation]
  );

  const handleEnqueue = useCallback(async () => {
    if (!selectedRepoId || !targetBranch) {
      return;
    }
    setIsEnqueuing(true);
    try {
      // The stream pushes the new row, so no refetch is needed here.
      await workspacesApi.create({
        name: null,
        repos: [{ repo_id: selectedRepoId, target_branch: targetBranch }],
      });
    } finally {
      setIsEnqueuing(false);
    }
  }, [selectedRepoId, targetBranch]);

  // Reuses the existing workspace action menu (archive/unarchive, editor, etc.)
  // instead of archiving on click — archiving runs the archive script and
  // cleans up the worktree, so it must not fire from an ambiguous icon.
  const handleOpenActions = useCallback((workspaceId: string) => {
    CommandBarDialog.show({ page: 'workspaceActions', workspaceId });
  }, []);

  const columnOrder: ColumnId[] = [
    'queued',
    'running',
    'errored',
    'done',
    'archived',
  ];

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full w-full">
        <p className="text-low">{t('states.loading')}</p>
      </div>
    );
  }

  return (
    <div className="h-full w-full overflow-auto bg-primary">
      {/* Drag is disabled board-wide: columns are derived, so a dropped card
          would snap straight back. KanbanProvider is still required to give the
          Droppable/Draggable primitives their context. */}
      <KanbanProvider onDragEnd={() => {}}>
        {columnOrder.map((columnId) => {
          const items = columns[columnId];
          return (
            <KanbanBoard key={columnId}>
              <KanbanHeader>
                <div className="border-t sticky border-b top-0 z-20 flex shrink-0 items-center justify-between gap-2 p-base bg-secondary">
                  <div className="flex items-center gap-2">
                    <div
                      className={`h-2 w-2 rounded-full shrink-0 ${COLUMN_DOT_CLASS[columnId]}`}
                    />
                    <p className="m-0 text-sm">
                      {t(`workspacesBoard.columns.${columnId}`)}
                    </p>
                    <span className="text-xs text-low">{items.length}</span>
                  </div>
                  {columnId === 'queued' && (
                    <div className="flex items-center gap-half min-w-0">
                      <RepoSelector
                        repos={repos}
                        selectedRepoId={selectedRepoId}
                        onRepoSelect={setSelectedRepoId}
                        placeholder={t('workspacesBoard.selectRepo')}
                        className="max-w-[10rem]"
                      />
                      <button
                        type="button"
                        onClick={() => void handleEnqueue()}
                        disabled={isEnqueuing || !targetBranch}
                        aria-label={t('workspacesBoard.enqueue')}
                        title={
                          targetBranch
                            ? t('workspacesBoard.enqueue')
                            : t('workspacesBoard.selectRepo')
                        }
                        className="p-half rounded-sm text-low hover:text-normal hover:bg-secondary transition-colors disabled:opacity-50"
                      >
                        <PlusIcon className="size-icon-base" weight="bold" />
                      </button>
                    </div>
                  )}
                </div>
              </KanbanHeader>
              <KanbanCards id={columnId}>
                {items.map((workspace, index) => (
                  <KanbanCard
                    key={workspace.id}
                    id={workspace.id}
                    name={workspace.name}
                    index={index}
                    dragDisabled
                    onClick={() => handleOpenWorkspace(workspace.id)}
                  >
                    <KanbanCardContent
                      displayId={workspace.branch}
                      // useWorkspaces falls back to the branch when a workspace
                      // has no name yet (its name is derived from the first
                      // prompt), which would print the branch twice per card.
                      title={
                        workspace.name === workspace.branch
                          ? t('workspacesBoard.untitled')
                          : workspace.name
                      }
                      priority={null}
                      tags={[]}
                      assignees={[]}
                      isLoading={workspace.isRunning}
                      pullRequests={prBadgesFor(workspace)}
                      onMoreActionsClick={() => handleOpenActions(workspace.id)}
                    />
                  </KanbanCard>
                ))}
                {items.length === 0 && (
                  <p className="p-base text-xs text-low">
                    {t('workspacesBoard.empty')}
                  </p>
                )}
              </KanbanCards>
            </KanbanBoard>
          );
        })}
      </KanbanProvider>
    </div>
  );
}
