import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';

import { listWasmVersions, pushWasmToHarbor, wasmDownloadUrl, type WasmEntry } from '../api/ntxBackendWasm';

function bytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
    return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

export function WasmListPage() {
    const [items, setItems] = useState<WasmEntry[]>([]);
    const [loading, setLoading] = useState<boolean>(false);
    const [error, setError] = useState<string | null>(null);

    const [pushRef, setPushRef] = useState<string>('');
    const [includeCatalog, setIncludeCatalog] = useState<boolean>(true);
    const [pushStatus, setPushStatus] = useState<string | null>(null);

    const sorted = useMemo(() => [...items].sort((a, b) => a.sha256.localeCompare(b.sha256)), [items]);

    async function refresh() {
        setLoading(true);
        setError(null);
        try {
            const data = await listWasmVersions();
            setItems(data);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setLoading(false);
        }
    }

    useEffect(() => {
        void refresh();
    }, []);

    async function pushOne(sha256: string) {
        if (!pushRef.trim()) {
            setPushStatus('missing ref');
            return;
        }
        setPushStatus('pushing...');
        try {
            const resp = await pushWasmToHarbor({ ref: pushRef.trim(), wasmSha256: sha256, includeCatalog });
            setPushStatus(`ok: ${resp.ref}`);
        } catch (e) {
            setPushStatus(e instanceof Error ? e.message : String(e));
        }
    }

    return (
        <div style={{ padding: 16 }}>
            <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
                <h1 style={{ margin: 0, fontSize: 18 }}>WASM Versions</h1>
                <Link to="/wasm/upload">Upload</Link>
                <span style={{ flex: 1 }} />
                <Link to="/builder">Builder</Link>
            </div>

            <div style={{ marginTop: 12, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
                <button onClick={() => void refresh()} disabled={loading}>
                    Refresh
                </button>
                {loading ? <span>Loading…</span> : null}
                {error ? <span style={{ color: 'crimson' }}>{error}</span> : null}
            </div>

            <div style={{ marginTop: 12, padding: 12, border: '1px solid #ddd', borderRadius: 6 }}>
                <div style={{ fontWeight: 600 }}>Push to Harbor</div>
                <div style={{ marginTop: 8, display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
                    <label style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                        <span>Ref</span>
                        <input
                            style={{ width: 360 }}
                            value={pushRef}
                            onChange={(e) => setPushRef(e.target.value)}
                            placeholder="192.168.31.138/ntx/executor:v0.0.1"
                        />
                    </label>
                    <label style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                        <input type="checkbox" checked={includeCatalog} onChange={(e) => setIncludeCatalog(e.target.checked)} />
                        <span>Include catalog</span>
                    </label>
                    {pushStatus ? <span>{pushStatus}</span> : null}
                </div>
                <div style={{ marginTop: 6, color: '#666', fontSize: 12 }}>
                    Click “Push” on a row to publish that wasm under the ref.
                </div>
            </div>

            <table style={{ width: '100%', marginTop: 12, borderCollapse: 'collapse' }}>
                <thead>
                    <tr>
                        <th style={{ textAlign: 'left', borderBottom: '1px solid #ddd', padding: '8px 6px' }}>sha256</th>
                        <th style={{ textAlign: 'left', borderBottom: '1px solid #ddd', padding: '8px 6px' }}>refs</th>
                        <th style={{ textAlign: 'right', borderBottom: '1px solid #ddd', padding: '8px 6px' }}>size</th>
                        <th style={{ textAlign: 'left', borderBottom: '1px solid #ddd', padding: '8px 6px' }}>actions</th>
                    </tr>
                </thead>
                <tbody>
                    {sorted.map((it) => (
                        <tr key={it.sha256}>
                            <td style={{ padding: '8px 6px', fontFamily: 'monospace' }}>{it.sha256}</td>
                            <td style={{ padding: '8px 6px', color: '#555' }}>
                                {it.refs.length === 0 ? (
                                    <span style={{ color: '#999' }}>—</span>
                                ) : it.refs.length === 1 ? (
                                    <span style={{ fontFamily: 'monospace' }}>{it.refs[0]}</span>
                                ) : (
                                    <span style={{ fontFamily: 'monospace' }}>{it.refs[0]}</span>
                                )}
                                {it.refs.length > 1 ? <span style={{ marginLeft: 8, color: '#999' }}>+{it.refs.length - 1}</span> : null}
                            </td>
                            <td style={{ padding: '8px 6px', textAlign: 'right' }}>{bytes(it.size_bytes)}</td>
                            <td style={{ padding: '8px 6px', display: 'flex', gap: 10, alignItems: 'center' }}>
                                <a href={wasmDownloadUrl(it.sha256)} target="_blank" rel="noreferrer">
                                    Download
                                </a>
                                <button onClick={() => void pushOne(it.sha256)}>Push</button>
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>

            {sorted.length === 0 && !loading ? <div style={{ marginTop: 12, color: '#666' }}>No wasm uploaded yet.</div> : null}
        </div>
    );
}
