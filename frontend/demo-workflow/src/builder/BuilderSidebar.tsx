import type { ActionsCatalog, ActionSummary } from '../types/catalog';
import { ActionPalette } from '../components/ActionPalette';
import type { NtxNodeType } from '../hooks/useWorkflowEditor';

export function BuilderSidebar(props: {
    catalog: ActionsCatalog | null;
    catalogError: string | null;
    catalogLoading: boolean;
    wasmCatalogs: Array<{ sha256: string; size_bytes: number; refs: string[] }>;
    selectedWasmSha256: string | null;
    onSelectWasmSha256: (sha256: string) => void;
    actions: ActionSummary[];
    onPickAction: (a: ActionSummary) => void;
    onAddQuickNode: (t: NtxNodeType) => void;
    errorCount: number;
    warningCount: number;
    exportBlockCount: number;
    onCopyJson: () => Promise<void>;
    onCopyScenarioYaml: () => Promise<void>;
}) {
    const { catalog, catalogError, catalogLoading } = props;

    return (
        <aside className="sidebar">
            <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                <h1 style={{ margin: 0 }}>Ntx Workflow Demo</h1>
            </div>

            <div className="muted" style={{ marginTop: 6, marginBottom: 12 }}>
                Builder
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
                    <strong>Export</strong>
                    <div style={{ display: 'flex', gap: 8 }}>
                        <span className="muted" style={{ alignSelf: 'center' }}>
                            {props.errorCount || props.warningCount
                                ? `${props.errorCount} error(s), ${props.warningCount} warning(s)`
                                : 'ok'}
                        </span>
                        <button onClick={props.onCopyJson}>Copy JSON</button>
                        <button disabled={props.exportBlockCount > 0} onClick={props.onCopyScenarioYaml}>
                            Copy scenario.yaml
                        </button>
                    </div>
                </div>

                <div className="muted" style={{ marginTop: 6 }}>
                    Copy graph JSON and scenario.yaml to clipboard.
                </div>

                {props.exportBlockCount > 0 ? (
                    <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>
                        Export is blocked ({props.exportBlockCount}). See Validation panel.
                    </div>
                ) : null}
            </div>
        </aside>
    );
}
