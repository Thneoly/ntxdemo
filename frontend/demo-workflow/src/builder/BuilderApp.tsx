import { useEffect, useMemo, useState } from 'react';
import ReactFlow, {
    Background,
    Controls,
    MiniMap,
    type Edge,
    type Node,
    type NodeMouseHandler,
    useReactFlow,
} from 'reactflow';

import './builder.css';

import type { ActionsCatalog, ActionSummary } from '../types/catalog';
import { nodeTypes } from '../components/nodeTypes';
import { getNtxNodeType, useWorkflowEditor, type NtxNodeType, type ValidationIssue } from '../hooks/useWorkflowEditor';
import { buildScenarioYaml } from '../workflow/export';
import { validateExportBlocking, validateGraph } from '../workflow/validate';

import {
    backendCatalogUrl,
    defaultCatalogRef,
    defaultBackendBaseUrl,
    loadWorkflow,
    saveWorkflow,
    type BackendWorkflowDraft,
} from '../api/ntxBackend';

import { getWasmGeneratedCatalog, listWasmVersions, type WasmEntry } from '../api/ntxBackendWasm';
import { getConfigBundle, listConfigBundles, type ConfigBundleSummary, type GetConfigBundleResp } from '../api/ntxBackendConfigBundles';

import { BuilderSidebar } from './BuilderSidebar';
import {
    createRunBundle,
    getRunBundleLogs,
    getRunBundleStatus,
    runRunBundle,
    stopRunBundle,
    type CreateRunBundleResp,
    type RunBundleLogsResp,
    type RunBundleStatusResp,
    type RunRunBundleResp,
} from '../api/ntxBackendRunBundles';

function toRecord(data: unknown): Record<string, unknown> {
    if (data && typeof data === 'object' && !Array.isArray(data)) {
        return data as Record<string, unknown>;
    }
    return {};
}

function stripUiFields(data: Record<string, unknown>): Record<string, unknown> {
    const out: Record<string, unknown> = { ...data };
    delete out._highlight;
    // Internal node actions injected by the editor.
    delete out._ntx;
    return out;
}

function safeJsonParseObject(input: string): { ok: true; value: Record<string, unknown> } | { ok: false; error: string } {
    try {
        const v: unknown = JSON.parse(input);
        if (!v || typeof v !== 'object' || Array.isArray(v)) {
            return { ok: false, error: 'must be a JSON object' };
        }
        return { ok: true, value: v as Record<string, unknown> };
    } catch (e) {
        return { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
}

function findActionDefaults(catalog: ActionsCatalog | null, call: string): Record<string, unknown> {
    if (!catalog) return {};
    const entry = catalog.actions.find((a) => a.summary.id === call);
    const defaultsJson = entry?.spec?.defaults_json;
    if (!defaultsJson) return {};
    const parsed = safeJsonParseObject(defaultsJson);
    return parsed.ok ? parsed.value : {};
}

// ValidationIssue / NtxNodeType / getNtxNodeType moved to useWorkflowEditor hook.

async function loadCatalog(): Promise<ActionsCatalog> {
    // The dev server serves from public/.
    const res = await fetch('/actions-catalog.json');
    if (!res.ok) {
        throw new Error(`failed to load /actions-catalog.json: ${res.status} ${res.statusText}`);
    }
    return (await res.json()) as ActionsCatalog;
}

async function loadCatalogFromUrl(url: string): Promise<ActionsCatalog> {
    const res = await fetch(url);
    if (!res.ok) {
        throw new Error(`failed to load ${url}: ${res.status} ${res.statusText}`);
    }
    return (await res.json()) as ActionsCatalog;
}

function summarizeActions(catalog: ActionsCatalog): ActionSummary[] {
    // Flatten to summaries; keep stable ordering.
    return catalog.actions
        .map((a) => a.summary)
        .sort((a, b) => a.id.localeCompare(b.id));
}

export default function BuilderApp() {
    const [runOutputOpen, setRunOutputOpen] = useState(false);
    const rf = useReactFlow();
    const [catalog, setCatalog] = useState<ActionsCatalog | null>(null);
    const [catalogError, setCatalogError] = useState<string | null>(null);
    const [catalogLoading, setCatalogLoading] = useState<boolean>(false);

    const [wasmCatalogs, setWasmCatalogs] = useState<WasmEntry[]>([]);
    const [selectedWasmSha256, setSelectedWasmSha256] = useState<string | null>(null);

    const [configBundles, setConfigBundles] = useState<ConfigBundleSummary[]>([]);
    const [configBundlesLoading, setConfigBundlesLoading] = useState<boolean>(false);
    const [configBundlesError, setConfigBundlesError] = useState<string | null>(null);
    const [selectedConfigBundleName, setSelectedConfigBundleName] = useState<string | null>(null);
    const [selectedConfigBundle, setSelectedConfigBundle] = useState<GetConfigBundleResp | null>(null);

    const [workflowId, setWorkflowId] = useState<string | null>(null);

    const {
        nodes,
        setNodes,
        edges,
        setEdges,
        onNodesChange,
        onEdgesChange,
        onConnect,
        selectedNodeId,
        setSelectedNodeId,
        selectedNode,
        uiWarning,
        setUiWarning,
        deleteSelectedNode,
        setNodeAsStart,
        withNodeActions,
    } = useWorkflowEditor();
    const selectedNodeData = useMemo(() => (selectedNode ? toRecord(selectedNode.data) : null), [selectedNode]);
    const selectedType = selectedNode ? getNtxNodeType(selectedNode) : null;
    const selectedIsAction = selectedType === 'action';
    const selectedIsWait = selectedType === 'wait';
    const selectedCall = selectedIsAction && selectedNodeData ? (selectedNodeData.call as string) : null;
    const [withText, setWithText] = useState<string>('{}');

    const [waitEvent, setWaitEvent] = useState<string>('packet-rx');
    const [waitMatchText, setWaitMatchText] = useState<string>('{}');
    const [waitError, setWaitError] = useState<string | null>(null);

    const [runPackaging, setRunPackaging] = useState<boolean>(false);
    const [runPackageError, setRunPackageError] = useState<string | null>(null);
    const [runPackageResult, setRunPackageResult] = useState<CreateRunBundleResp | null>(null);

    const [runStarting, setRunStarting] = useState<boolean>(false);
    const [runStartError, setRunStartError] = useState<string | null>(null);
    const [runStartResult, setRunStartResult] = useState<RunRunBundleResp | null>(null);

    const [runStatusLoading, setRunStatusLoading] = useState<boolean>(false);
    const [runStatus, setRunStatus] = useState<RunBundleStatusResp | null>(null);
    const [runLogsLoading, setRunLogsLoading] = useState<boolean>(false);
    const [runLogs, setRunLogs] = useState<RunBundleLogsResp | null>(null);
    const [runControlError, setRunControlError] = useState<string | null>(null);

    const actions = useMemo(() => (catalog ? summarizeActions(catalog) : []), [catalog]);

    const hydrateDraftNodes = (draft: BackendWorkflowDraft['nodes']) => {
        return draft.map((n) => ({
            id: n.id,
            type: n.type,
            position: { x: n.position.x, y: n.position.y },
            data: withNodeActions(n.id, n.data),
        })) as Node[];
    };

    const hydrateDraftEdges = (draft: BackendWorkflowDraft['edges']) => {
        return draft.map((e) => ({
            id: e.id,
            source: e.source,
            target: e.target,
        })) as Edge[];
    };

    useEffect(() => {
        // Default catalog source on first mount.
        // Priority: uploaded wasm-generated catalogs (from backend) -> env ref -> public/actions-catalog.json.
        setCatalogLoading(true);
        listWasmVersions()
            .then(async (entries) => {
                setWasmCatalogs(entries);
                if (entries.length) {
                    const sha = entries[0]!.sha256;
                    setSelectedWasmSha256(sha);
                    try {
                        const c = await getWasmGeneratedCatalog(sha);
                        setCatalog(c);
                        setCatalogError(null);
                        return;
                    } catch (e) {
                        // Backend might be running an older build without the wasm-catalog endpoint.
                        setCatalogError(e instanceof Error ? e.message : String(e));
                        // Fall through to env/public fallback catalog.
                    }
                }

                const ref = defaultCatalogRef();
                const baseUrl = defaultBackendBaseUrl();
                const preferredUrl = ref ? backendCatalogUrl(ref, { baseUrl }) : null;
                const c = preferredUrl ? await loadCatalogFromUrl(preferredUrl) : await loadCatalog();
                setCatalog(c);
                // Keep any earlier error message if we hit it; otherwise clear.
                setCatalogError((prev) => prev);
            })
            .catch((e) => {
                setCatalog(null);
                setCatalogError(e instanceof Error ? e.message : String(e));
            })
            .finally(() => setCatalogLoading(false));
    }, []);

    useEffect(() => {
        // Load available config bundles (for app.yaml wasm linkage).
        setConfigBundlesLoading(true);
        setConfigBundlesError(null);
        listConfigBundles()
            .then((items) => {
                setConfigBundles(items);
            })
            .catch((e) => {
                setConfigBundles([]);
                setConfigBundlesError(e instanceof Error ? e.message : String(e));
            })
            .finally(() => setConfigBundlesLoading(false));
    }, []);

    useEffect(() => {
        // Default to the newest config bundle once loaded.
        // Backend sorts ascending by name, so pick the last.
        if (selectedConfigBundleName) return;
        if (!configBundles.length) return;
        void selectConfigBundle(configBundles[configBundles.length - 1]!.name);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [configBundles, selectedConfigBundleName]);

    const selectConfigBundle = async (name: string) => {
        setSelectedConfigBundleName(name);
        setSelectedConfigBundle(null);
        try {
            const b = await getConfigBundle(name);
            setSelectedConfigBundle(b);
            setConfigBundlesError(null);
        } catch (e) {
            setConfigBundlesError(e instanceof Error ? e.message : String(e));
        }
    };

    const selectWasmCatalog = async (sha256: string) => {
        setSelectedWasmSha256(sha256);
        setCatalogLoading(true);
        try {
            const c = await getWasmGeneratedCatalog(sha256);
            setCatalog(c);
            setCatalogError(null);
        } catch (e) {
            setCatalogError(e instanceof Error ? e.message : String(e));
        } finally {
            setCatalogLoading(false);
        }
    };

    useEffect(() => {
        // Auto-load a workflow draft from backend when possible.
        // Priority: URL param ?wf=... -> localStorage.
        const params = new URLSearchParams(window.location.search);
        const wfFromUrl = params.get('wf');
        const stored = window.localStorage.getItem('ntx.demo_workflow.workflow_id');
        const initialId = wfFromUrl?.trim() || stored?.trim() || null;
        if (!initialId) return;

        loadWorkflow(initialId)
            .then((draft) => {
                setWorkflowId(initialId);
                window.localStorage.setItem('ntx.demo_workflow.workflow_id', initialId);
                setNodes(hydrateDraftNodes(draft.nodes));
                setEdges(hydrateDraftEdges(draft.edges));
                if (draft.viewport) {
                    rf.setViewport(draft.viewport, { duration: 0 });
                }
            })
            .catch((e) => {
                setUiWarning(`backend load failed: ${e instanceof Error ? e.message : String(e)}`);
            });
    }, []);

    useEffect(() => {
        // Auto-save draft to backend (debounced). No new UI required.
        const timer = window.setTimeout(() => {
            const draft: BackendWorkflowDraft = {
                schema_version: 1,
                nodes: nodes.map((n: Node) => ({
                    id: n.id,
                    type: String(n.type ?? 'default'),
                    position: { x: n.position.x, y: n.position.y },
                    data: stripUiFields(toRecord(n.data)),
                })),
                edges: edges.map((e: Edge) => ({
                    id: e.id,
                    source: e.source,
                    target: e.target,
                })),
                viewport: rf.getViewport(),
            };

            saveWorkflow(draft, { id: workflowId ?? undefined })
                .then((resp) => {
                    if (!workflowId) {
                        setWorkflowId(resp.id);
                        window.localStorage.setItem('ntx.demo_workflow.workflow_id', resp.id);
                    }
                })
                .catch((e) => {
                    setUiWarning(`backend save failed: ${e instanceof Error ? e.message : String(e)}`);
                });
        }, 800);

        return () => window.clearTimeout(timer);
    }, [nodes, edges, rf, workflowId]);

    // Keep editor text in sync when selection changes.
    useEffect(() => {
        if (!selectedNodeData) {
            setWithText('{}');
            setWaitEvent('packet-rx');
            setWaitMatchText('{}');
            setWaitError(null);
            return;
        }

        if (selectedType === 'action') {
            const withObj = toRecord(selectedNodeData.with);
            setWithText(JSON.stringify(withObj, null, 2));
        }

        if (selectedType === 'wait') {
            const onObj = toRecord(selectedNodeData.on);
            const evt = onObj.event;
            setWaitEvent(typeof evt === 'string' && evt.length ? evt : 'packet-rx');
            const matchObj = toRecord(toRecord(onObj).match);
            setWaitMatchText(JSON.stringify(matchObj, null, 2));
            setWaitError(null);
        }
    }, [selectedNodeId]);

    // Keyboard shortcuts + core graph mutations are owned by useWorkflowEditor.

    const addActionNode = (action: ActionSummary) => {
        const id = `n-${crypto.randomUUID()}`;

        const existingRefs = new Set(
            nodes
                .map((n: Node) => toRecord(n.data).action_ref)
                .filter((v: unknown): v is string => typeof v === 'string')
        );
        let actionRef = action.id;
        if (existingRefs.has(actionRef)) {
            let i = 2;
            while (existingRefs.has(`${action.id}#${i}`)) i++;
            actionRef = `${action.id}#${i}`;
        }

        const newNode: Node = {
            id,
            type: 'action',
            position: {
                x: 80 + nodes.length * 30,
                y: 80 + nodes.length * 30,
            },
            data: withNodeActions(id, {
                label: actionRef,
                ntx_node_type: 'action',
                action_ref: actionRef,
                call: action.id,
                with: findActionDefaults(catalog, action.id),
            }),
        };
        setNodes((ns: Node[]) => [...ns, newNode]);
    };

    const addQuickNode = (t: NtxNodeType) => {
        const id = `n-${crypto.randomUUID()}`;
        const base: Node = {
            id,
            type: t,
            position: {
                x: 80 + nodes.length * 30,
                y: 80 + nodes.length * 30,
            },
            data: withNodeActions(id, {
                label: t,
                ntx_node_type: t,
            }),
        };

        if (t === 'wait') {
            base.data = {
                ...toRecord(base.data),
                label: 'wait',
                on: { event: 'packet-rx', match: {} },
            };
        }

        setNodes((ns: Node[]) => [...ns, base]);
    };

    const scenarioYaml = useMemo(
        () =>
            buildScenarioYaml({
                workflowName: 'workflow-demo',
                nodes,
                edges,
                options: {
                    catalog: catalog ?? undefined,
                    includeDemoScaffold: true,
                },
            }),
        [catalog, nodes, edges]
    );

    const validationIssues = useMemo(() => validateGraph(nodes, edges, { catalog: catalog ?? undefined }), [catalog, nodes, edges]);
    const exportBlockingIssues = useMemo(
        () => validateExportBlocking(nodes, edges, { catalog: catalog ?? undefined }),
        [catalog, nodes, edges]
    );
    const warningCount = validationIssues.filter((i: ValidationIssue) => i.level === 'warning').length;
    const errorCount = validationIssues.filter((i: ValidationIssue) => i.level === 'error').length;
    const exportBlockCount = exportBlockingIssues.filter((i: ValidationIssue) => i.level === 'error').length;

    const packageRunBundle = async () => {
        setRunPackageError(null);
        setRunPackageResult(null);
        setRunStartError(null);
        setRunStartResult(null);
        setRunStatus(null);
        setRunLogs(null);
        setRunControlError(null);
        setUiWarning(null);

        if (!selectedConfigBundleName) {
            setRunPackageError('Select a Config Bundle');
            return;
        }
        if (!selectedWasmSha256) {
            setRunPackageError('Select a Catalog (From uploaded WASM)');
            return;
        }
        if (exportBlockCount > 0) {
            setUiWarning('Packaging blocked: fix blocking issues first (see Validation).');
            return;
        }

        setRunPackaging(true);
        try {
            const resp = await createRunBundle({
                config_bundle: selectedConfigBundleName,
                wasm_sha256: selectedWasmSha256,
                scenario_yaml: scenarioYaml,
            });
            setRunPackageResult(resp);
        } catch (e) {
            setRunPackageError(e instanceof Error ? e.message : String(e));
        } finally {
            setRunPackaging(false);
        }
    };

    const runPackagedBundle = async () => {
        setRunStartError(null);
        setRunStartResult(null);
        setRunControlError(null);
        setUiWarning(null);

        if (!runPackageResult) {
            setRunStartError('Package a run bundle first.');
            return;
        }

        setRunStarting(true);
        try {
            const resp = await runRunBundle(runPackageResult.id);
            setRunStartResult(resp);
            // Kick an immediate refresh for status/logs.
            try {
                const [st, lg] = await Promise.all([
                    getRunBundleStatus(runPackageResult.id),
                    getRunBundleLogs(runPackageResult.id),
                ]);
                setRunStatus(st);
                setRunLogs(lg);
            } catch {
                // Ignore refresh errors here; they'll be visible via manual refresh.
            }
        } catch (e) {
            setRunStartError(e instanceof Error ? e.message : String(e));
        } finally {
            setRunStarting(false);
        }
    };

    const refreshRunInfo = async () => {
        setRunControlError(null);
        if (!runPackageResult) {
            setRunControlError('Package a run bundle first.');
            return;
        }

        setRunStatusLoading(true);
        setRunLogsLoading(true);
        try {
            const [st, lg] = await Promise.all([
                getRunBundleStatus(runPackageResult.id),
                getRunBundleLogs(runPackageResult.id),
            ]);
            setRunStatus(st);
            setRunLogs(lg);
        } catch (e) {
            setRunControlError(e instanceof Error ? e.message : String(e));
        } finally {
            setRunStatusLoading(false);
            setRunLogsLoading(false);
        }
    };

    const stopRunningBundle = async () => {
        setRunControlError(null);
        if (!runPackageResult) {
            setRunControlError('Package a run bundle first.');
            return;
        }

        try {
            await stopRunBundle(runPackageResult.id);
            await refreshRunInfo();
        } catch (e) {
            setRunControlError(e instanceof Error ? e.message : String(e));
        }
    };

    useEffect(() => {
        // When a run bundle is packaged (or changed), load initial status/logs.
        if (!runPackageResult) return;
        void refreshRunInfo();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [runPackageResult?.id]);

    useEffect(() => {
        // Poll while running (lightweight demo UX).
        if (!runPackageResult?.id) return;
        if (!runStatus?.running) return;
        const id = runPackageResult.id;
        const t = window.setInterval(() => {
            void (async () => {
                try {
                    const [st, lg] = await Promise.all([getRunBundleStatus(id), getRunBundleLogs(id)]);
                    setRunStatus(st);
                    setRunLogs(lg);
                } catch {
                    // Ignore transient polling errors.
                }
            })();
        }, 1500);
        return () => window.clearInterval(t);
    }, [runPackageResult?.id, runStatus?.running]);

    const applyWaitToSelected = () => {
        if (!selectedNodeId) return;
        const parsedMatch = safeJsonParseObject(waitMatchText);
        if (!parsedMatch.ok) {
            setWaitError(parsedMatch.error);
            return;
        }
        if (!waitEvent || !waitEvent.trim()) {
            setWaitError('event is required');
            return;
        }
        setWaitError(null);
        setNodes((ns: Node[]) =>
            ns.map((n) => {
                if (n.id !== selectedNodeId) return n;
                const data = toRecord(n.data);
                return {
                    ...n,
                    data: {
                        ...data,
                        ntx_node_type: 'wait',
                        on: {
                            event: waitEvent.trim(),
                            match: parsedMatch.value,
                        },
                    },
                };
            })
        );
    };

    return (
        <div className="builderGrid">
            <BuilderSidebar
                catalog={catalog}
                catalogError={catalogError}
                catalogLoading={catalogLoading}
                wasmCatalogs={wasmCatalogs}
                selectedWasmSha256={selectedWasmSha256}
                onSelectWasmSha256={(sha) => void selectWasmCatalog(sha)}

                configBundles={configBundles}
                configBundlesLoading={configBundlesLoading}
                configBundlesError={configBundlesError}
                selectedConfigBundleName={selectedConfigBundleName}
                selectedConfigBundle={selectedConfigBundle}
                onSelectConfigBundleName={(name) => void selectConfigBundle(name)}

                actions={actions}
                onPickAction={addActionNode}
                onAddQuickNode={addQuickNode}
                exportBlockCount={exportBlockCount}

                runPackaging={runPackaging}
                runPackageError={runPackageError}
                runPackageResult={runPackageResult ? { id: runPackageResult.id, dir: runPackageResult.dir } : null}
                runOutputUrl={
                    runPackageResult
                        ? (() => {
                            const baseUrl = defaultBackendBaseUrl().replace(/\/$/, '');
                            const id = encodeURIComponent(runPackageResult.id);
                            return `${baseUrl}/api/v1/run-bundles/${id}/output`;
                        })()
                        : null
                }
                runOutputOpen={runOutputOpen}
                runStarting={runStarting}
                runStartError={runStartError}
                runStartResult={runStartResult ? { pid: runStartResult.pid } : null}

                runStatusLoading={runStatusLoading}
                runStatus={runStatus ? { running: runStatus.running, pid: runStatus.pid, exit_code: runStatus.exit_code } : null}
                runLogsLoading={runLogsLoading}
                runLogs={runLogs ? { stdout: runLogs.stdout, stderr: runLogs.stderr, truncated: runLogs.truncated } : null}
                runControlError={runControlError}
                onPackage={packageRunBundle}
                onRun={() => void runPackagedBundle()}
                onStop={() => void stopRunningBundle()}
                onRefreshRunInfo={() => void refreshRunInfo()}
                onToggleRunOutput={() => setRunOutputOpen((v) => !v)}
            />

            <main className="builderContent">
                <ReactFlow
                    nodes={nodes}
                    edges={edges}
                    nodeTypes={nodeTypes}
                    onNodesChange={onNodesChange}
                    onEdgesChange={onEdgesChange}
                    onConnect={onConnect}
                    onNodeClick={((_evt: unknown, n: Node) => setSelectedNodeId(n.id)) as unknown as NodeMouseHandler}
                    fitView
                >
                    <Background />
                    <MiniMap />
                    <Controls />
                </ReactFlow>
            </main>
            <aside className="builderRightbar">
                <div className="toolbar" style={{ justifyContent: 'space-between', marginBottom: 10 }}>
                    <h1 style={{ fontSize: 16, margin: 0 }}>Inspector</h1>
                </div>

                {validationIssues.length || exportBlockingIssues.length ? (
                    <div className="card">
                        <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                            <strong>Validation</strong>
                            <span className="muted">{exportBlockCount ? 'blocked' : errorCount ? 'errors' : 'warnings'}</span>
                        </div>
                        <div style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 6 }}>
                            {uiWarning ? (
                                <div style={{ fontSize: 12, color: '#92400e' }}>{uiWarning}</div>
                            ) : null}
                            {exportBlockingIssues.length ? (
                                <>
                                    <div className="muted" style={{ marginTop: 4 }}>
                                        Export blocking
                                    </div>
                                    {exportBlockingIssues.slice(0, 8).map((i: ValidationIssue, idx: number) => (
                                        <div
                                            key={`blk-${idx}`}
                                            style={{
                                                fontSize: 12,
                                                color: '#b91c1c',
                                                display: 'flex',
                                                gap: 8,
                                                alignItems: 'center',
                                            }}
                                        >
                                            <span style={{ flex: 1 }}>{i.message}</span>
                                            {i.nodeId ? (
                                                <button
                                                    style={{ fontSize: 12, padding: '2px 6px' }}
                                                    onClick={() => {
                                                        setSelectedNodeId(i.nodeId!);
                                                        // Center on the node if it exists.
                                                        const n = nodes.find((x: Node) => x.id === i.nodeId) ?? null;
                                                        if (n) {
                                                            rf.fitView({ nodes: [n], duration: 250, padding: 0.4 });
                                                        }

                                                        // Temporary highlight.
                                                        const id = i.nodeId!;
                                                        setNodes((ns: Node[]) =>
                                                            ns.map((x: Node) => {
                                                                if (x.id !== id) return x;
                                                                return {
                                                                    ...x,
                                                                    data: {
                                                                        ...(toRecord(x.data) as Record<string, unknown>),
                                                                        _highlight: true,
                                                                    },
                                                                };
                                                            })
                                                        );
                                                        window.setTimeout(() => {
                                                            setNodes((ns: Node[]) =>
                                                                ns.map((x: Node) => {
                                                                    if (x.id !== id) return x;
                                                                    const d = toRecord(x.data);
                                                                    if (!('_highlight' in d)) return x;
                                                                    const { _highlight: _ignored, ...rest } = d as Record<string, unknown>;
                                                                    return { ...x, data: rest };
                                                                })
                                                            );
                                                        }, 2000);
                                                    }}
                                                >
                                                    定位
                                                </button>
                                            ) : null}
                                        </div>
                                    ))}
                                </>
                            ) : null}
                            {validationIssues.length ? (
                                <div className="muted" style={{ marginTop: exportBlockingIssues.length ? 10 : 4 }}>
                                    Edit-time warnings/errors
                                </div>
                            ) : null}
                            {validationIssues.slice(0, 8).map((i: ValidationIssue, idx: number) => (
                                <div
                                    key={idx}
                                    style={{
                                        fontSize: 12,
                                        color: i.level === 'error' ? '#b91c1c' : '#92400e',
                                    }}
                                >
                                    {i.message}
                                </div>
                            ))}
                            {validationIssues.length > 8 ? (
                                <div className="muted">(+{validationIssues.length - 8} more)</div>
                            ) : null}
                        </div>
                    </div>
                ) : null}

                <div className="card">
                    <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                        <strong>Selection</strong>
                        <span className="muted">{selectedNode ? selectedNode.id : 'none'}</span>
                    </div>

                    {selectedNode ? (
                        <div style={{ display: 'flex', gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
                            <button onClick={() => setNodeAsStart(selectedNode.id)} disabled={getNtxNodeType(selectedNode) === 'start'}>
                                Set as Start
                            </button>
                            <button onClick={deleteSelectedNode}>Delete</button>
                        </div>
                    ) : null}

                    {!selectedNode ? (
                        <div className="muted" style={{ marginTop: 8 }}>
                            Click a node to edit.
                        </div>
                    ) : null}

                    {selectedNode && selectedIsAction ? (
                        <>
                            <div className="fieldLabel">call</div>
                            <div className="input small">
                                <code>{selectedCall}</code>
                            </div>

                            <div className="fieldLabel">with (read-only)</div>
                            <textarea
                                className="jsonOutput"
                                style={{ minHeight: 180 }}
                                value={withText}
                                readOnly
                            />

                            <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
                                <button
                                    onClick={() => {
                                        if (!selectedNodeId || !selectedCall) return;
                                        const d = findActionDefaults(catalog, selectedCall);
                                        setNodes((ns: Node[]) =>
                                            ns.map((n) => {
                                                if (n.id !== selectedNodeId) return n;
                                                const data = toRecord(n.data);
                                                return {
                                                    ...n,
                                                    data: {
                                                        ...data,
                                                        with: d,
                                                    },
                                                };
                                            })
                                        );
                                        setWithText(JSON.stringify(d, null, 2));
                                    }}
                                >
                                    Reset to defaults
                                </button>
                            </div>
                        </>
                    ) : null}

                    {selectedNode && selectedIsWait ? (
                        <>
                            <div className="fieldLabel">on.event</div>
                            <input
                                className="input"
                                value={waitEvent}
                                onChange={(e) => setWaitEvent(e.target.value)}
                                placeholder="packet-rx"
                            />

                            <div className="fieldLabel">on.match (JSON object)</div>
                            <textarea
                                className="jsonOutput"
                                style={{ minHeight: 160 }}
                                value={waitMatchText}
                                onChange={(e) => setWaitMatchText(e.target.value)}
                            />

                            {waitError ? (
                                <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>{waitError}</div>
                            ) : null}

                            <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
                                <button onClick={applyWaitToSelected}>Apply</button>
                                <button
                                    onClick={() => {
                                        setWaitEvent('packet-rx');
                                        setWaitMatchText('{}');
                                        setWaitError(null);
                                    }}
                                >
                                    Reset
                                </button>
                            </div>
                        </>
                    ) : null}

                    {selectedNode && !selectedIsAction ? (
                        <div className="muted" style={{ marginTop: 8 }}>
                            (v0) Action and wait nodes have editable fields.
                        </div>
                    ) : null}
                </div>
            </aside>

            {runOutputOpen ? (
                <aside className="builderOutputDrawer" aria-label="Run output">
                    <div className="builderOutputDrawerHeader">
                        <strong>Run output</strong>
                        <button onClick={() => setRunOutputOpen(false)}>Close</button>
                    </div>
                    <div className="builderOutputDrawerBody">
                        {runPackageResult ? (
                            <iframe
                                title="Run output"
                                src={(() => {
                                    const baseUrl = defaultBackendBaseUrl().replace(/\/$/, '');
                                    const id = encodeURIComponent(runPackageResult.id);
                                    return `${baseUrl}/api/v1/run-bundles/${id}/output`;
                                })()}
                                className="builderOutputDrawerFrame"
                            />
                        ) : (
                            <div className="muted" style={{ fontSize: 12 }}>No run bundle yet.</div>
                        )}
                    </div>
                </aside>
            ) : null}
        </div>
    );
}
