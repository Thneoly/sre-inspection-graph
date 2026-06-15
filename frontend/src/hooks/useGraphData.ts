import { useQuery } from '@tanstack/react-query';
import type { AxiosResponse } from 'axios';
import type { GraphResponse } from '../api/client';

export function useGraphData<T extends GraphResponse>(
  key: string,
  fetcher: () => Promise<AxiosResponse<T>>,
  params?: Record<string, unknown>
) {
  return useQuery({
    queryKey: [key, params],
    queryFn: async () => {
      const response = await fetcher();
      return response.data;
    },
    staleTime: 30_000,
    refetchInterval: 60_000,
  });
}
