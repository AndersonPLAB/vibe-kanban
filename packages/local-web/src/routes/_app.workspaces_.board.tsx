import { createFileRoute } from '@tanstack/react-router';
import { WorkspacesBoardContainer } from '@/features/kanban/ui/WorkspacesBoardContainer';

export const Route = createFileRoute('/_app/workspaces_/board')({
  component: WorkspacesBoardContainer,
});
