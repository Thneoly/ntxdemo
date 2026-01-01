import type { ActionsCatalog, ActionSummary } from '../types/catalog';
import { ActionPalette } from '../components/ActionPalette';
import type { NtxNodeType } from '../hooks/useWorkflowEditor';
import type { ConfigBundleSummary, GetConfigBundleResp } from '../api/ntxBackendConfigBundles';

export function BuilderSidebar(props: {
    catalog: ActionsCatalog | null;
    catalogError: string | null;
    catalogLoading: boolean;
    wasmCatalogs: Array<{ sha256: string; size_bytes: number; refs: string[] }>;
    selectedWasmSha256: string | null;
    onSelectWasmSha256: (sha256: string) => void;

    configBundles: ConfigBundleSummary[];
    configBundlesLoading: boolean;
    configBundlesError: string | null;
    selectedConfigBundleName: string | null;
    selectedConfigBundle: GetConfigBundleResp | null;
    onSelectConfigBundleName: (name: string) => void;
    actions: ActionSummary[];
    onPickAction: (a: ActionSummary) => void;
    onAddQuickNode: (t: NtxNodeType) => void;
    exportBlockCount: number;

    runPackaging: boolean;
    runPackageError: string | null;
    runPackageResult: { id: string; dir: string } | null;
    runOutputUrl: string | null;
    runOutputOpen: boolean;
    runStarting: boolean;
    runStartError: string | null;
    runStartResult: { pid: number } | null;

    runStatusLoading: boolean;
    runStatus: { running: boolean; pid: number | null; exit_code: number | null } | null;
    runLogsLoading: boolean;
    runLogs: { stdout: string; stderr: string; truncated: boolean } | null;
    runControlError: string | null;
    onPackage: () => Promise<void>;
    onRun: () => void;
    onStop: () => void;
    onRefreshRunInfo: () => void;
    onToggleRunOutput: () => void;
}) {
    const { catalog, catalogError, catalogLoading } = props;

    return (
        <aside className="builderSidebar">
            <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                <h1 style={{ margin: 0 }}>Ntx Workflow Demo</h1>
            </div>

            <div className="muted" style={{ marginTop: 6, marginBottom: 12 }}>
                Builder
            </div>

            <div className="card">
                <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                    <strong>Config Bundle</strong>
                    <span className="muted">
                        {props.configBundlesLoading
                            ? 'loading…'
                            : props.configBundles.length
                                ? `${props.configBundles.length} item(s)`
                                : 'empty'}
                    </span>
                </div>

                <div style={{ marginTop: 10 }}>
                    <select
                        style={{ width: '100%' }}
                        value={props.selectedConfigBundleName ?? ''}
                        onChange={(e) => {
                            const v = e.target.value;
                            if (v) props.onSelectConfigBundleName(v);
                        }}
                        disabled={!props.configBundles.length}
                    >
                        <option value="" disabled>
                            Select a bundle…
                        </option>
                        {props.configBundles.map((b) => (
                            <option key={b.name} value={b.name}>
                                {b.name}
                            </option>
                        ))}
                    </select>
                </div>

                {props.configBundlesError ? (
                    <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>{props.configBundlesError}</div>
                ) : null}

                {props.selectedConfigBundle ? (
                    <div className="muted" style={{ marginTop: 10, fontSize: 12 }}>
                        <div>
                            dir: <code>{props.selectedConfigBundle.dir}</code>
                        </div>

                        {props.selectedConfigBundle.scheduler_wasm_parse_error ? (
                            <div style={{ marginTop: 6, color: '#b91c1c' }}>
                                app.yaml parse error: <code>{props.selectedConfigBundle.scheduler_wasm_parse_error}</code>
                            </div>
                        ) : null}

                        {props.selectedConfigBundle.scheduler_wasm ? (
                            <>
                                <div style={{ marginTop: 6 }}>
                                    component_path:{' '}
                                    <code>{props.selectedConfigBundle.scheduler_wasm.component_path ?? '—'}</code>
                                </div>
                                <div style={{ marginTop: 4 }}>
                                    config_dir: <code>{props.selectedConfigBundle.scheduler_wasm.config_dir ?? '—'}</code>
                                </div>
                                <div style={{ marginTop: 4 }}>
                                    entry_candidates:{' '}
                                    <code>
                                        {props.selectedConfigBundle.scheduler_wasm.entry_candidates.length
                                            ? props.selectedConfigBundle.scheduler_wasm.entry_candidates.join(', ')
                                            : '—'}
                                    </code>
                                </div>
                            </>
                        ) : (
                            <div style={{ marginTop: 6 }}>scheduler.wasm: <code>—</code></div>
                        )}
                    </div>
                ) : null}
            </div>

            <div className="card">
                <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                    <strong>Catalog</strong>
                    <span className="muted">
                        {catalogLoading ? 'loading…' : catalog ? `schema_version=${catalog.schema_version}` : 'not loaded'}
                    </span>
                </div>

                {props.wasmCatalogs.length ? (
                    <div style={{ marginTop: 10 }}>
                        <div className="muted" style={{ marginBottom: 6 }}>
                            From uploaded WASM
                        </div>
                        <select
                            style={{ width: '100%' }}
                            value={props.selectedWasmSha256 ?? ''}
                            onChange={(e) => {
                                const v = e.target.value;
                                if (v) props.onSelectWasmSha256(v);
                            }}
                        >
                            <option value="" disabled>
                                Select a wasm…
                            </option>
                            {props.wasmCatalogs.map((w) => {
                                const labelRef = w.refs[0] ? ` — ${w.refs[0]}` : '';
                                return (
                                    <option key={w.sha256} value={w.sha256}>
                                        {w.sha256.slice(0, 12)}…{labelRef}
                                    </option>
                                );
                            })}
                        </select>
                    </div>
                ) : null}

                {catalogError ? <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>{catalogError}</div> : null}
                {catalog?.executor_component?.digest ? (
                    <div className="muted" style={{ marginTop: 8 }}>
                        digest: <code>{catalog.executor_component.digest}</code>
                    </div>
                ) : null}
            </div>

            <ActionPalette actions={props.actions} onPick={props.onPickAction} />

            <div className="card">
                <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                    <strong>Quick Nodes</strong>
                    <span className="muted">add</span>
                </div>
                <div style={{ display: 'flex', gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
                    <button onClick={() => props.onAddQuickNode('start')}>Start</button>
                    <button onClick={() => props.onAddQuickNode('wait')}>Wait</button>
                    <button onClick={() => props.onAddQuickNode('end')}>End</button>
                </div>
            </div>

            <div className="card">
                <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                    <strong>Run</strong>
                    <div className="builderRunActions">
                        <button disabled={props.runPackaging} onClick={props.onPackage}>
                            {props.runPackaging ? 'Packaging…' : 'Package'}
                        </button>
                        <button disabled={!props.runPackageResult || props.runStarting} onClick={props.onRun}>
                            {props.runStarting ? 'Starting…' : 'Run'}
                        </button>
                        <button disabled={!props.runStatus?.running} onClick={props.onStop}>
                            Stop
                        </button>
                        <button disabled={!props.runOutputUrl} onClick={props.onToggleRunOutput}>
                            {props.runOutputOpen ? 'View (open)' : 'View'}
                        </button>
                    </div>
                </div>

                <div className="muted" style={{ marginTop: 6 }}>
                    Package config + scenario.yaml + wasm to backend.
                </div>

                <div className="muted" style={{ marginTop: 8, fontSize: 12 }}>
                    <div>
                        config bundle: <code>{props.selectedConfigBundleName ?? '—'}</code>
                    </div>
                    <div style={{ marginTop: 4 }}>
                        wasm: <code>{props.selectedWasmSha256 ? `${props.selectedWasmSha256.slice(0, 12)}…` : '—'}</code>
                    </div>
                    <div style={{ marginTop: 4 }}>
                        export blocking: <code>{props.exportBlockCount > 0 ? `${props.exportBlockCount} blocked` : 'ok'}</code>
                    </div>
                </div>

                {props.runPackageError ? (
                    <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>{props.runPackageError}</div>
                ) : null}

                {props.runStartError ? (
                    <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>{props.runStartError}</div>
                ) : null}

                {props.runControlError ? (
                    <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>{props.runControlError}</div>
                ) : null}

                {props.runPackageResult ? (
                    <div className="muted" style={{ marginTop: 8, fontSize: 12 }}>
                        output: <code>{props.runPackageResult.dir}/output</code>
                    </div>
                ) : null}

                {props.runStartResult ? (
                    <div className="muted" style={{ marginTop: 6, fontSize: 12 }}>
                        started pid: <code>{props.runStartResult.pid}</code>
                    </div>
                ) : null}

                <div style={{ marginTop: 10, display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <strong style={{ fontSize: 12 }}>Status</strong>
                    <button disabled={!props.runPackageResult || props.runStatusLoading || props.runLogsLoading} onClick={props.onRefreshRunInfo}>
                        {props.runStatusLoading || props.runLogsLoading ? 'Refreshing…' : 'Refresh'}
                    </button>
                </div>

                <div className="muted" style={{ marginTop: 6, fontSize: 12 }}>
                    <div>
                        running:{' '}
                        <code>
                            {props.runStatus ? (props.runStatus.running ? 'true' : 'false') : props.runPackageResult ? 'unknown' : '—'}
                        </code>
                    </div>
                    <div style={{ marginTop: 4 }}>
                        pid: <code>{props.runStatus?.pid ?? '—'}</code>
                    </div>
                    <div style={{ marginTop: 4 }}>
                        exit_code: <code>{props.runStatus?.exit_code ?? '—'}</code>
                    </div>
                </div>

                {props.runLogs ? (
                    <div style={{ marginTop: 10 }}>
                        <div className="muted" style={{ fontSize: 12 }}>
                            logs tail{props.runLogs.truncated ? ' (truncated)' : ''}:
                        </div>
                        <div style={{ marginTop: 6 }}>
                            <div className="muted" style={{ fontSize: 12, marginBottom: 4 }}>
                                stdout
                            </div>
                            <pre
                                style={{
                                    margin: 0,
                                    fontSize: 11,
                                    whiteSpace: 'pre-wrap',
                                    maxHeight: 160,
                                    overflow: 'auto',
                                }}
                            >
                                {props.runLogs.stdout || '—'}
                            </pre>
                        </div>
                        <div style={{ marginTop: 10 }}>
                            <div className="muted" style={{ fontSize: 12, marginBottom: 4 }}>
                                stderr
                            </div>
                            <pre
                                style={{
                                    margin: 0,
                                    fontSize: 11,
                                    whiteSpace: 'pre-wrap',
                                    maxHeight: 160,
                                    overflow: 'auto',
                                }}
                            >
                                {props.runLogs.stderr || '—'}
                            </pre>
                        </div>
                    </div>
                ) : null}

                {props.exportBlockCount > 0 ? (
                    <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>
                        Packaging is blocked ({props.exportBlockCount}). See Validation panel.
                    </div>
                ) : null}
            </div>
        </aside>
    );
}
