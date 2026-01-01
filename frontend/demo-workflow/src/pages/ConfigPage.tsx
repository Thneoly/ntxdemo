import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';

import { putConfigBundle, type PutConfigBundleResp } from '../api/ntxBackendConfigBundles';

const DEFAULT_APP_YAML = `# Unified app configuration for Ntx.

kernel:
  # Kernel config path (NIC + resource pools).
  config_path: "config/config.yaml"

scheduler:
  no_idle_wait: false
  wasm:
    component_path: "./component/wac/scheduler-composed.wasm"
    config_dir: "./component/conf/udp-echo-minimal"
    entry_candidates:
      - "run"
`;

const DEFAULT_CONFIG_YAML = `nic:
  iface: "ntx0"
resource:
  path: "config/resource/resources.yaml"

# Optional: host-side packet capture (pcap, Ethernet/L2 frames).
capture:
    enabled: true
    dir: "./pcap"
    rotate_max_bytes: 104857600  # 100 MiB
    rotate_interval_secs: 60
`;

const DEFAULT_RESOURCES_YAML = `ipv4:
  - name: default
    cidr: "10.0.0.0/28"
    exclude: ["10.0.0.1", "10.0.0.2", "10.0.0.3"]

mac:
  - name: default
    start: "02:00:00:00:00:10"
    end:   "02:00:00:00:00:19"

udp_port:
  - name: default
    start: 40000
    end: 40009
`;

function defaultBundleName(): string {
    const d = new Date();
    const pad = (n: number) => String(n).padStart(2, '0');
    return `bundle-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
}

export function ConfigPage() {
    const [name, setName] = useState<string>(defaultBundleName());
    const [appYaml, setAppYaml] = useState<string>(DEFAULT_APP_YAML);
    const [configYaml, setConfigYaml] = useState<string>(DEFAULT_CONFIG_YAML);
    const [resourcesYaml, setResourcesYaml] = useState<string>(DEFAULT_RESOURCES_YAML);

    const [saving, setSaving] = useState<boolean>(false);
    const [msg, setMsg] = useState<{ kind: 'success' | 'error' | 'info'; text: string } | null>(null);
    const [resp, setResp] = useState<PutConfigBundleResp | null>(null);

    const runHint = useMemo(() => {
        if (!resp) return null;
        return {
            dir: `cargo run -p Ntx -- --config ${resp.dir}`,
        };
    }, [resp]);

    async function onSave() {
        const trimmed = name.trim();
        if (!trimmed) {
            setMsg({ kind: 'error', text: 'bundle name is required' });
            return;
        }

        setSaving(true);
        setMsg({ kind: 'info', text: 'saving…' });
        setResp(null);
        try {
            const r = await putConfigBundle({
                name: trimmed,
                appYaml,
                configYaml,
                resourcesYaml,
            });
            setResp(r);
            setMsg({ kind: 'success', text: `saved to ${r.dir}` });
        } catch (e) {
            setMsg({ kind: 'error', text: e instanceof Error ? e.message : String(e) });
        } finally {
            setSaving(false);
        }
    }

    return (
        <div className="page">
            <div className="pageHeader">
                <h1 className="pageTitle">Config Bundle</h1>
                <div className="pageActions">
                    <Link to="/">Home</Link>
                    <Link to="/builder">Builder</Link>
                    <Link to="/wasm">WASM</Link>
                </div>
            </div>

            <div className="card" style={{ maxWidth: 980 }}>
                <div style={{ fontWeight: 600 }}>Bundle name</div>
                <div className="muted" style={{ marginTop: 6 }}>
                    Saved under <code>${'{'}DATA_DIR{'}'}/config-bundles/&lt;name&gt;/config/...</code> (name must match <code>[A-Za-z0-9._-]</code>).
                </div>
                <div style={{ marginTop: 10, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
                    <input
                        style={{ width: 520, maxWidth: '100%' }}
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="bundle name (e.g. udp-echo-dev)"
                    />
                    <button disabled={saving} onClick={() => void onSave()}>
                        {saving ? 'Saving…' : 'Save to backend'}
                    </button>
                </div>

                {msg ? (
                    <div style={{ marginTop: 10 }}>
                        <pre
                            className="mono"
                            style={{
                                margin: 0,
                                fontSize: 12,
                                whiteSpace: 'pre-wrap',
                                overflowWrap: 'anywhere',
                                color: msg.kind === 'error' ? '#b91c1c' : msg.kind === 'success' ? '#0a7a0a' : '#374151',
                            }}
                        >
                            {msg.text}
                        </pre>
                    </div>
                ) : null}

                {resp ? (
                    <div className="muted" style={{ marginTop: 10, fontSize: 12 }}>
                        <div>Run hint (dir): <code>{runHint?.dir}</code></div>
                    </div>
                ) : null}
            </div>

            <div className="card" style={{ maxWidth: 980 }}>
                <div style={{ fontWeight: 600 }}>config/app.yaml</div>
                <textarea
                    className="jsonOutput"
                    style={{ minHeight: 220, marginTop: 8 }}
                    value={appYaml}
                    onChange={(e) => setAppYaml(e.target.value)}
                />
            </div>

            <div className="card" style={{ maxWidth: 980 }}>
                <div style={{ fontWeight: 600 }}>config/config.yaml</div>
                <textarea
                    className="jsonOutput"
                    style={{ minHeight: 140, marginTop: 8 }}
                    value={configYaml}
                    onChange={(e) => setConfigYaml(e.target.value)}
                />
            </div>

            <div className="card" style={{ maxWidth: 980 }}>
                <div style={{ fontWeight: 600 }}>config/resource/resources.yaml</div>
                <textarea
                    className="jsonOutput"
                    style={{ minHeight: 180, marginTop: 8 }}
                    value={resourcesYaml}
                    onChange={(e) => setResourcesYaml(e.target.value)}
                />
            </div>
        </div>
    );
}
