import type { ActionsCatalog } from '../types/catalog';

export type BackendWorkflowDraft = {
    schema_version: number;
    nodes: Array<{
        id: string;
        type: string;
        position: { x: number; y: number };
        data: Record<string, unknown>;
    }>;
    edges: Array<{
        id: string;
        source: string;
        target: string;
    }>;
    viewport?: { x: number; y: number; zoom: number };
};

export type BackendClientOptions = {
    baseUrl?: string;
};

function envString(key: string): string | undefined {
    const v = (import.meta as unknown as { env?: Record<string, string | undefined> }).env?.[key];
    return typeof v === 'string' && v.trim().length ? v.trim() : undefined;
}

export function defaultBackendBaseUrl(): string {
    return envString('VITE_NTX_BACKEND_URL') ?? 'http://127.0.0.1:9090';
}

export function defaultCatalogRef(): string | undefined {
    return envString('VITE_NTX_CATALOG_REF');
}

function joinUrl(baseUrl: string, path: string): string {
    if (baseUrl.endsWith('/')) baseUrl = baseUrl.slice(0, -1);
    if (!path.startsWith('/')) path = `/${path}`;
    return `${baseUrl}${path}`;
}

async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
    const res = await fetch(url, init);
    if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(`HTTP ${res.status} ${res.statusText}: ${text}`);
    }
    return (await res.json()) as T;
}

export function backendCatalogUrl(ref: string, opts: BackendClientOptions = {}): string {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = new URL(joinUrl(baseUrl, '/api/v1/catalog'));
    url.searchParams.set('ref', ref);
    return url.toString();
}

export async function getCatalog(ref: string, opts: BackendClientOptions = {}): Promise<ActionsCatalog> {
    return await fetchJson<ActionsCatalog>(backendCatalogUrl(ref, opts));
}

export async function saveWorkflow(
    graph: BackendWorkflowDraft,
    opts: BackendClientOptions & { id?: string } = {}
): Promise<{ id: string }> {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = joinUrl(baseUrl, '/api/v1/workflows');
    const body = {
        ...(opts.id ? { id: opts.id } : {}),
        graph,
    };
    return await fetchJson<{ id: string }>(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
    });
}

export async function loadWorkflow(id: string, opts: BackendClientOptions = {}): Promise<BackendWorkflowDraft> {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = joinUrl(baseUrl, `/api/v1/workflows/${encodeURIComponent(id)}`);
    const resp = await fetchJson<{ id: string; graph: BackendWorkflowDraft }>(url);
    return resp.graph;
}
