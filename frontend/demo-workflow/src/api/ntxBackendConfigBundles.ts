import { defaultBackendBaseUrl } from './ntxBackend';

export type PutConfigBundleResp = {
    name: string;
    dir: string;
    app_yaml_path: string;
    config_yaml_path: string;
    resources_yaml_path: string;
};

export type ConfigBundleSummary = {
    name: string;
    dir: string;
    app_yaml_path: string;
};

export type SchedulerWasmConfigExtract = {
    component_path?: string;
    config_dir?: string;
    entry_candidates: string[];
};

export type GetConfigBundleResp = {
    name: string;
    dir: string;
    app_yaml_path: string;
    config_yaml_path: string;
    resources_yaml_path: string;

    app_yaml: string;
    config_yaml: string;
    resources_yaml: string;

    scheduler_wasm?: SchedulerWasmConfigExtract;
    scheduler_wasm_parse_error?: string;
};

function baseUrl(): string {
    return defaultBackendBaseUrl();
}

export async function putConfigBundle(opts: {
    name: string;
    appYaml: string;
    configYaml: string;
    resourcesYaml: string;
}): Promise<PutConfigBundleResp> {
    const res = await fetch(`${baseUrl()}/api/v1/config-bundles`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
            name: opts.name,
            app_yaml: opts.appYaml,
            config_yaml: opts.configYaml,
            resources_yaml: opts.resourcesYaml,
        }),
    });

    if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(`failed to save config bundle: ${res.status} ${res.statusText}${text ? `: ${text}` : ''}`);
    }

    return (await res.json()) as PutConfigBundleResp;
}

export async function listConfigBundles(): Promise<ConfigBundleSummary[]> {
    const res = await fetch(`${baseUrl()}/api/v1/config-bundles`);
    if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(
            `failed to list config bundles: ${res.status} ${res.statusText}${text ? `: ${text}` : ''}`
        );
    }
    return (await res.json()) as ConfigBundleSummary[];
}

export async function getConfigBundle(name: string): Promise<GetConfigBundleResp> {
    const res = await fetch(`${baseUrl()}/api/v1/config-bundles/${encodeURIComponent(name)}`);
    if (!res.ok) {
        const text = await res.text().catch(() => '');
        throw new Error(
            `failed to get config bundle: ${res.status} ${res.statusText}${text ? `: ${text}` : ''}`
        );
    }
    return (await res.json()) as GetConfigBundleResp;
}
