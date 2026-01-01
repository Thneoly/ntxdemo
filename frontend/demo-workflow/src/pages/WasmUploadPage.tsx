import { useState } from 'react';
import { Link, useNavigate } from 'react-router-dom';

import { uploadWasm } from '../api/ntxBackendWasm';

export function WasmUploadPage() {
    const nav = useNavigate();
    const [file, setFile] = useState<File | null>(null);
    const [status, setStatus] = useState<string | null>(null);

    async function onUpload() {
        if (!file) {
            setStatus('select a .wasm file first');
            return;
        }
        setStatus('uploading...');
        try {
            const resp = await uploadWasm(file);
            setStatus(`ok: ${resp.sha256}`);
            // Jump back to list so user can push/download.
            setTimeout(() => nav('/wasm'), 250);
        } catch (e) {
            setStatus(e instanceof Error ? e.message : String(e));
        }
    }

    return (
        <div className="page">
            <div className="pageHeader">
                <h1 className="pageTitle">Upload WASM</h1>
                <div className="pageActions">
                    <Link to="/wasm">Back to list</Link>
                    <Link to="/builder">Builder</Link>
                </div>
            </div>

            <div>
                <input
                    type="file"
                    accept=".wasm,application/wasm"
                    onChange={(e) => setFile(e.target.files?.[0] ?? null)}
                />
            </div>

            <div style={{ marginTop: 12 }}>
                <button onClick={() => void onUpload()} disabled={!file}>
                    Upload
                </button>
                {status ? <span style={{ marginLeft: 10 }}>{status}</span> : null}
            </div>

            <div style={{ marginTop: 8, color: '#666', fontSize: 12 }}>
                The backend stores the uploaded wasm by sha256 under <code>.ntx-backend/wasm/</code>.
            </div>
        </div>
    );
}
