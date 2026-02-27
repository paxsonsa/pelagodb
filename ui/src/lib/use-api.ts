import { DependencyList, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { normalizeApiError } from "@/lib/api";

export function useApiAction<TArgs extends unknown[], TResult>(
  action: (...args: TArgs) => Promise<TResult>,
) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<TResult | null>(null);

  const execute = useCallback(
    async (...args: TArgs): Promise<TResult | null> => {
      setLoading(true);
      setError(null);
      try {
        const result = await action(...args);
        setData(result);
        return result;
      } catch (cause) {
        setError(normalizeApiError(cause));
        return null;
      } finally {
        setLoading(false);
      }
    },
    [action],
  );

  return {
    loading,
    error,
    data,
    setData,
    execute,
  };
}

export function useApiQuery<TResult>(
  query: (signal: AbortSignal) => Promise<TResult>,
  deps: DependencyList,
  enabled = true,
) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [data, setData] = useState<TResult | null>(null);
  const controllerRef = useRef<AbortController | null>(null);

  const run = useCallback(async () => {
    if (!enabled) {
      return null;
    }

    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;

    setLoading(true);
    setError(null);

    try {
      const result = await query(controller.signal);
      if (!controller.signal.aborted) {
        setData(result);
      }
      return result;
    } catch (cause) {
      if (!controller.signal.aborted) {
        setError(normalizeApiError(cause));
      }
      return null;
    } finally {
      if (!controller.signal.aborted) {
        setLoading(false);
      }
    }
  }, [enabled, query]);

  useEffect(() => {
    void run();

    return () => {
      controllerRef.current?.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return useMemo(
    () => ({ loading, error, data, setData, refetch: run }),
    [data, error, loading, run],
  );
}
