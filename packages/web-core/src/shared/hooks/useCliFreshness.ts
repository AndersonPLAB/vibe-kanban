import { useQuery } from '@tanstack/react-query';
import { configApi } from '@/shared/lib/api';

/** Fetches the CLI freshness report once on app load. Backend owns the 24h cache. */
export function useCliFreshness() {
  return useQuery({
    queryKey: ['cli-freshness'],
    queryFn: configApi.getCliFreshness,
    staleTime: Infinity,
    retry: false,
    refetchOnMount: false,
    refetchOnWindowFocus: false,
  });
}
