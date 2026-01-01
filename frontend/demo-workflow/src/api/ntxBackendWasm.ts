import { defaultBackendBaseUrl } from './ntxBackend';
import type { ActionsCatalog } from '../types/catalog';

export type WasmEntry = {
    sha256: string;
    size_bytes: number;
    refs: string[];
};

export type UploadWasmResp = {
    sha256: string;
    size_bytes: number;
    file: string;
};

export type PushWasmResp = {
    ref: string;
    wasm_sha256: string;
    artifact_type: string;
    included_catalog: boolean;
};

function baseUrl(): string {
    return defaultBackendBaseUrl();
}

export async function listWasmVersions(): Promise<WasmEntry[]> {
    const res = await fetch(`${baseUrl()}/api/v1/wasm`);
    if (!res.ok) throw new Error(`failed to list wasm: ${res.status} ${res.statusText}`);
    return (await res.json()) as WasmEntry[];
}

export function wasmDownloadUrl(sha256: string): string {
    return `${baseUrl()}/api/v1/wasm/${encodeURIComponent(sha256)}`;
}

export async function uploadWasm(file: File): Promise<UploadWasmResp> {
    const form = new FormData();
    form.append('file', file);

    const res = await fetch(`${baseUrl()}/api/v1/wasm/upload`, {
        method: 'POST',
        body: form,
    });
    if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(`upload failed: ${res.status} ${res.statusText}${text ? `: ${text}` : ''}`);
    }
    return (await res.json()) as UploadWasmResp;
}

export async function pushWasmToHarbor(opts: {
    ref: string;
    wasmSha256: string;
    includeCatalog: boolean;
    artifactType?: string;
}): Promise<PushWasmResp> {
    const res = await fetch(`${baseUrl()}/api/v1/wasm/push`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
            ref: opts.ref,
            wasm_sha256: opts.wasmSha256,
            include_catalog: opts.includeCatalog,
            artifact_type: opts.artifactType,
        }),
    });
    if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(`push failed: ${res.status} ${res.statusText}${text ? `: ${text}` : ''}`);
    }
    return (await res.json()) as PushWasmResp;
}

export async function getWasmGeneratedCatalog(sha256: string): Promise<ActionsCatalog> {
    const res = await fetch(`${baseUrl()}/api/v1/wasm/${encodeURIComponent(sha256)}/catalog`);
    if (!res.ok) {
        const text = await res.text().catch(() => '');
        if (res.status === 404 && !text.trim()) {
            throw new Error(
                `failed to load wasm catalog: 404 Not Found (backend missing /api/v1/wasm/{sha256}/catalog; please rebuild/restart ntx-backend)`
            );
        }
        throw new Error(
            `failed to load wasm catalog: ${res.status} ${res.statusText}${text ? `: ${text}` : ''}`
        );
    }
    return (await res.json()) as ActionsCatalog;
}
