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
    const [pushingSha, setPushingSha] = useState<string | null>(null);
    const [pushMessage, setPushMessage] = useState<{ kind: 'info' | 'success' | 'error'; text: string } | null>(null);

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
            setPushMessage({ kind: 'error', text: 'missing ref' });
            return;
        }
        setPushMessage({ kind: 'info', text: `pushing ${sha256}...` });
        setPushingSha(sha256);
        try {
            const resp = await pushWasmToHarbor({ ref: pushRef.trim(), wasmSha256: sha256, includeCatalog: true });
            setPushMessage({ kind: 'success', text: `ok: ${resp.ref}` });
        } catch (e) {
            const msg = e instanceof Error ? e.message : String(e);
            setPushMessage({ kind: 'error', text: msg });
        } finally {
            setPushingSha((cur) => (cur === sha256 ? null : cur));
        }
    }

    return (
        <div className="page">
            <div className="pageTitleRow">
                <h1 className="pageTitle">
                    WASM Versions
                </h1>
                <div className="navLinks">
                    <Link to="/wasm/upload">Upload</Link>
                </div>
            </div>

            <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
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
                            placeholder="192.168.31.138/ntx/executor:v0.0.1 (no http/https)"
                        />
                    </label>
                </div>
                <div style={{ marginTop: 6, color: '#666', fontSize: 12 }}>
                    Click “Push” on a row to publish that wasm + catalog.json under the ref.
                </div>
                <div style={{ marginTop: 6, color: '#666', fontSize: 12 }}>
                    Ref format: <span className="mono">&lt;registry&gt;/&lt;repo&gt;:tag</span> (example above).
                </div>
                {pushMessage ? (
                    <div
                        style={{
                            marginTop: 10,
                            padding: '8px 10px',
                            borderRadius: 6,
                            border: '1px solid #ddd',
                            background: '#fafafa',
                        }}
                    >
                        <div
                            style={{
                                fontSize: 12,
                                color:
                                    pushMessage.kind === 'error'
                                        ? 'crimson'
                                        : pushMessage.kind === 'success'
                                            ? '#0a7a0a'
                                            : '#555',
                                fontWeight: 600,
                            }}
                        >
                            {pushMessage.kind === 'error'
                                ? 'Push failed'
                                : pushMessage.kind === 'success'
                                    ? 'Push succeeded'
                                    : 'Push'}
                        </div>
                        <pre
                            className="mono"
                            style={{
                                margin: '6px 0 0 0',
                                fontSize: 12,
                                color: '#444',
                                whiteSpace: 'pre-wrap',
                                overflowWrap: 'anywhere',
                                maxHeight: 220,
                                overflow: 'auto',
                            }}
                        >
                            {pushMessage.text}
                        </pre>
                    </div>
                ) : null}
            </div>

            <table className="table" style={{ marginTop: 12 }}>
                <thead>
                    <tr>
                        <th>sha256</th>
                        <th>refs</th>
                        <th style={{ textAlign: 'right' }}>size</th>
                        <th>actions</th>
                    </tr>
                </thead>
                <tbody>
                    {sorted.map((it) => (
                        <tr key={it.sha256}>
                            <td className="mono">{it.sha256}</td>
                            <td style={{ color: '#555' }}>
                                {it.refs.length === 0 ? (
                                    <span style={{ color: '#999' }}>—</span>
                                ) : it.refs.length === 1 ? (
                                    <span className="mono">{it.refs[0]}</span>
                                ) : (
                                    <span className="mono">{it.refs[0]}</span>
                                )}
                                {it.refs.length > 1 ? <span style={{ marginLeft: 8, color: '#999' }}>+{it.refs.length - 1}</span> : null}
                            </td>
                            <td style={{ textAlign: 'right' }}>{bytes(it.size_bytes)}</td>
                            <td>
                                <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
                                    <a href={wasmDownloadUrl(it.sha256)} target="_blank" rel="noreferrer">
                                        Download
                                    </a>
                                    <button
                                        onClick={() => void pushOne(it.sha256)}
                                        disabled={!pushRef.trim() || pushingSha === it.sha256}
                                    >
                                        {pushingSha === it.sha256 ? 'Pushing…' : 'Push'}
                                    </button>
                                </div>
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>

            {sorted.length === 0 && !loading ? <div style={{ marginTop: 12, color: '#666' }}>No wasm uploaded yet.</div> : null}
        </div>
    );
}
