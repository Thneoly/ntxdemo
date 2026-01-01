import { defaultBackendBaseUrl } from './ntxBackend';

export type PutConfigBundleResp = {
    name: string;
    dir: string;
    app_yaml_path: string;
    config_yaml_path: string;
    resources_yaml_path: string;
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
