import type { BackendClientOptions } from './ntxBackend';
import { defaultBackendBaseUrl } from './ntxBackend';

export type CreateRunBundleReq = {
    id?: string;
    config_bundle: string;
    wasm_sha256: string;
    scenario_yaml: string;
};

export type CreateRunBundleResp = {
    id: string;
    dir: string;
    config_dir: string;
    scenario_yaml_path: string;
    wasm_path: string;

    // Added in later backend builds.
    catalog_path?: string;
    scheduler_composed_wasm_path?: string;
};

export type RunRunBundleResp = {
    id: string;
    pid: number;
    command: string[];
    run_dir: string;

    // Added in later backend builds.
    stdout_path?: string;
    stderr_path?: string;
};

export type RunBundleStatusResp = {
    id: string;
    running: boolean;
    pid: number | null;
    exit_code: number | null;
    command: string[] | null;
    run_dir: string;
    stdout_path: string;
    stderr_path: string;
};

export type RunBundleStopResp = {
    id: string;
    stopped: boolean;
};

export type RunBundleLogsResp = {
    id: string;
    stdout: string;
    stderr: string;
    stdout_path: string;
    stderr_path: string;
    truncated: boolean;
};

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

export async function createRunBundle(body: CreateRunBundleReq, opts: BackendClientOptions = {}): Promise<CreateRunBundleResp> {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = joinUrl(baseUrl, '/api/v1/run-bundles');
    return await fetchJson<CreateRunBundleResp>(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
    });
}

export async function runRunBundle(id: string, opts: BackendClientOptions = {}): Promise<RunRunBundleResp> {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = joinUrl(baseUrl, `/api/v1/run-bundles/${encodeURIComponent(id)}/run`);
    return await fetchJson<RunRunBundleResp>(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
    });
}

export async function getRunBundleStatus(id: string, opts: BackendClientOptions = {}): Promise<RunBundleStatusResp> {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = joinUrl(baseUrl, `/api/v1/run-bundles/${encodeURIComponent(id)}/status`);
    return await fetchJson<RunBundleStatusResp>(url);
}

export async function stopRunBundle(id: string, opts: BackendClientOptions = {}): Promise<RunBundleStopResp> {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = joinUrl(baseUrl, `/api/v1/run-bundles/${encodeURIComponent(id)}/stop`);
    return await fetchJson<RunBundleStopResp>(url, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
    });
}

export async function getRunBundleLogs(id: string, opts: BackendClientOptions = {}): Promise<RunBundleLogsResp> {
    const baseUrl = opts.baseUrl ?? defaultBackendBaseUrl();
    const url = joinUrl(baseUrl, `/api/v1/run-bundles/${encodeURIComponent(id)}/logs`);
    return await fetchJson<RunBundleLogsResp>(url);
}
