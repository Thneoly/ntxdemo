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
import 'reactflow/dist/style.css';

import type { ActionsCatalog, ActionSummary } from './types/catalog';
import { ActionPalette } from './components/ActionPalette';
import { nodeTypes } from './components/nodeTypes';
import { getNtxNodeType, useWorkflowEditor, type NtxNodeType, type ValidationIssue } from './hooks/useWorkflowEditor';
import { buildScenarioYaml } from './workflow/export';
import { validateExportBlocking, validateGraph } from './workflow/validate';
import type { WorkflowExport } from './workflow/types';

import { parse as parseYaml } from 'yaml';

import {
    backendCatalogUrl,
    defaultCatalogRef,
    defaultBackendBaseUrl,
    loadWorkflow,
    saveWorkflow,
    type BackendWorkflowDraft,
} from './api/ntxBackend';

import { Link } from 'react-router-dom';

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

function safeJsonParseAny(input: string): { ok: true; value: unknown } | { ok: false; error: string } {
    try {
        return { ok: true, value: JSON.parse(input) as unknown };
    } catch (e) {
        return { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
}

function safeParseYamlOrJsonObject(input: string): { ok: true; value: Record<string, unknown> } | { ok: false; error: string } {
    const trimmed = input.trim();
    if (!trimmed) return { ok: false, error: 'empty input' };

    // Heuristic: if it *looks* like JSON, parse as JSON first.
    // Otherwise try YAML, which can also parse JSON but has different error messages.
    const looksJson = trimmed.startsWith('{') || trimmed.startsWith('[') || trimmed === 'null' || trimmed === 'true' || trimmed === 'false' || /^-?\d/.test(trimmed);
    if (looksJson) {
        const parsed = safeJsonParseAny(trimmed);
        if (!parsed.ok) return { ok: false, error: `invalid JSON: ${parsed.error}` };
        const v = parsed.value;
        if (!v || typeof v !== 'object' || Array.isArray(v)) return { ok: false, error: 'template must be a YAML/JSON object (mapping)' };
        return { ok: true, value: v as Record<string, unknown> };
    }

    try {
        const v: unknown = parseYaml(trimmed);
        if (!v || typeof v !== 'object' || Array.isArray(v)) return { ok: false, error: 'template must be a YAML/JSON object (mapping)' };
        return { ok: true, value: v as Record<string, unknown> };
    } catch (e) {
        return { ok: false, error: `invalid YAML: ${e instanceof Error ? e.message : String(e)}` };
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

type CatalogSource =
    | { kind: 'default' }
    | { kind: 'url'; url: string }
    | { kind: 'file'; fileName: string };

async function parseCatalogJson(text: string): Promise<ActionsCatalog> {
    try {
        return JSON.parse(text) as ActionsCatalog;
    } catch (e) {
        throw new Error(`invalid JSON: ${e instanceof Error ? e.message : String(e)}`);
    }
}

async function loadCatalogFromUrl(url: string): Promise<ActionsCatalog> {
    const res = await fetch(url);
    if (!res.ok) {
        throw new Error(`failed to load ${url}: ${res.status} ${res.statusText}`);
    }
    return (await res.json()) as ActionsCatalog;
}

async function loadCatalogFromFile(file: File): Promise<ActionsCatalog> {
    const text = await file.text();
    return await parseCatalogJson(text);
}

function summarizeActions(catalog: ActionsCatalog): ActionSummary[] {
    // Flatten to summaries; keep stable ordering.
    return catalog.actions
        .map((a) => a.summary)
        .sort((a, b) => a.id.localeCompare(b.id));
}

export default function App() {
    const rf = useReactFlow();
    const [catalog, setCatalog] = useState<ActionsCatalog | null>(null);
    const [catalogError, setCatalogError] = useState<string | null>(null);

    const [catalogSource, setCatalogSource] = useState<CatalogSource>({ kind: 'default' });
    const [catalogUrl, setCatalogUrl] = useState<string>('');
    const [catalogLoading, setCatalogLoading] = useState<boolean>(false);

    const [workflowId, setWorkflowId] = useState<string | null>(null);
    const [backendStatus, setBackendStatus] = useState<string | null>(null);

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
    const [withError, setWithError] = useState<string | null>(null);

    const [waitEvent, setWaitEvent] = useState<string>('packet-rx');
    const [waitMatchText, setWaitMatchText] = useState<string>('{}');
    const [waitError, setWaitError] = useState<string | null>(null);

    // Scenario scaffold template injection (general builder): user can paste a base scenario.
    // Accept YAML or JSON; most users will paste a scenario_demo.yaml here.
    const [useScaffoldTemplate, setUseScaffoldTemplate] = useState<boolean>(false);
    const [scaffoldText, setScaffoldText] = useState<string>('');
    const [scaffoldError, setScaffoldError] = useState<string | null>(null);
    const scaffoldObject = useMemo(() => {
        if (!useScaffoldTemplate) return undefined;
        const trimmed = scaffoldText.trim();
        if (!trimmed) return undefined;
        const parsed = safeParseYamlOrJsonObject(trimmed);
        return parsed.ok ? parsed.value : undefined;
    }, [useScaffoldTemplate, scaffoldText]);

    useEffect(() => {
        if (!useScaffoldTemplate) {
            setScaffoldError(null);
            return;
        }
        const trimmed = scaffoldText.trim();
        if (!trimmed) {
            setScaffoldError(null);
            return;
        }
        const parsed = safeParseYamlOrJsonObject(trimmed);
        setScaffoldError(parsed.ok ? null : parsed.error);
    }, [useScaffoldTemplate, scaffoldText]);

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
        // Default catalog load on first mount.
        // Prefer backend catalog URL when env provides a catalog ref; fallback to public/actions-catalog.json.
        const ref = defaultCatalogRef();
        const baseUrl = defaultBackendBaseUrl();
        const preferredUrl = ref ? backendCatalogUrl(ref, { baseUrl }) : null;

        if (preferredUrl) {
            setCatalogUrl(preferredUrl);
            void reloadCatalog({ kind: 'url', url: preferredUrl });
            return;
        }

        setCatalogLoading(true);
        loadCatalog()
            .then((c) => {
                setCatalog(c);
                setCatalogError(null);
                setCatalogSource({ kind: 'default' });
            })
            .catch((e) => {
                setCatalog(null);
                setCatalogError(e instanceof Error ? e.message : String(e));
            })
            .finally(() => setCatalogLoading(false));
    }, []);

    useEffect(() => {
        // Auto-load a workflow draft from backend when possible.
        // Priority: URL param ?wf=... -> localStorage.
        const params = new URLSearchParams(window.location.search);
        const wfFromUrl = params.get('wf');
        const stored = window.localStorage.getItem('ntx.demo_workflow.workflow_id');
        const initialId = wfFromUrl?.trim() || stored?.trim() || null;
        if (!initialId) return;

        setBackendStatus('loading workflow…');
        loadWorkflow(initialId)
            .then((draft) => {
                setWorkflowId(initialId);
                window.localStorage.setItem('ntx.demo_workflow.workflow_id', initialId);
                setNodes(hydrateDraftNodes(draft.nodes));
                setEdges(hydrateDraftEdges(draft.edges));
                if (draft.viewport) {
                    rf.setViewport(draft.viewport, { duration: 0 });
                }
                setBackendStatus(`loaded workflow ${initialId}`);
            })
            .catch((e) => {
                setBackendStatus(`backend load failed: ${e instanceof Error ? e.message : String(e)}`);
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
                    setBackendStatus(`saved workflow ${workflowId ?? resp.id}`);
                })
                .catch((e) => {
                    setBackendStatus(`backend save failed: ${e instanceof Error ? e.message : String(e)}`);
                });
        }, 800);

        return () => window.clearTimeout(timer);
    }, [nodes, edges, rf, workflowId]);

    const reloadCatalog = async (src: CatalogSource) => {
        setCatalogLoading(true);
        setCatalogError(null);
        try {
            if (src.kind === 'default') {
                const c = await loadCatalog();
                setCatalog(c);
                setCatalogSource(src);
                return;
            }
            if (src.kind === 'url') {
                const url = src.url.trim();
                if (!url) throw new Error('URL is empty');
                const c = await loadCatalogFromUrl(url);
                setCatalog(c);
                setCatalogSource(src);
                return;
            }
            // file source is handled by file chooser event (it carries the file)
            throw new Error('file source must be loaded via file picker');
        } catch (e) {
            setCatalog(null);
            setCatalogError(e instanceof Error ? e.message : String(e));
        } finally {
            setCatalogLoading(false);
        }
    };

    const onPickCatalogFile = async (file: File | null) => {
        if (!file) return;
        setCatalogLoading(true);
        setCatalogError(null);
        try {
            const c = await loadCatalogFromFile(file);
            setCatalog(c);
            setCatalogSource({ kind: 'file', fileName: file.name });
        } catch (e) {
            setCatalog(null);
            setCatalogError(e instanceof Error ? e.message : String(e));
        } finally {
            setCatalogLoading(false);
        }
    };

    // Keep editor text in sync when selection changes.
    useEffect(() => {
        if (!selectedNodeData) {
            setWithText('{}');
            setWithError(null);
            setWaitEvent('packet-rx');
            setWaitMatchText('{}');
            setWaitError(null);
            return;
        }

        if (selectedType === 'action') {
            const withObj = toRecord(selectedNodeData.with);
            setWithText(JSON.stringify(withObj, null, 2));
            setWithError(null);
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

    const exported: WorkflowExport = useMemo(
        () => ({
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
        }),
        [nodes, edges]
    );

    const scenarioYaml = useMemo(
        () =>
            buildScenarioYaml({
                workflowName: 'workflow-demo',
                nodes,
                edges,
                options: {
                    catalog: catalog ?? undefined,
                    includeDemoScaffold: !useScaffoldTemplate,
                    scaffold: scaffoldObject,
                },
            }),
        [catalog, nodes, edges, scaffoldObject, useScaffoldTemplate]
    );

    const validationIssues = useMemo(() => validateGraph(nodes, edges, { catalog: catalog ?? undefined }), [catalog, nodes, edges]);
    const exportBlockingIssues = useMemo(
        () => validateExportBlocking(nodes, edges, { catalog: catalog ?? undefined }),
        [catalog, nodes, edges]
    );
    const warningCount = validationIssues.filter((i: ValidationIssue) => i.level === 'warning').length;
    const errorCount = validationIssues.filter((i: ValidationIssue) => i.level === 'error').length;
    const exportBlockCount = exportBlockingIssues.filter((i: ValidationIssue) => i.level === 'error').length;

    // scaffoldError is maintained by the YAML/JSON parsing effect near scaffoldObject.

    const applyWithToSelected = () => {
        if (!selectedNodeId) return;
        const parsed = safeJsonParseObject(withText);
        if (!parsed.ok) {
            setWithError(parsed.error);
            return;
        }
        setWithError(null);
        setNodes((ns: Node[]) =>
            ns.map((n) => {
                if (n.id !== selectedNodeId) return n;
                const data = toRecord(n.data);
                return {
                    ...n,
                    data: {
                        ...data,
                        with: parsed.value,
                    },
                };
            })
        );
    };

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
        <div className="app">
            <aside className="sidebar">
                <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                    <h1 style={{ margin: 0 }}>Ntx Workflow Demo</h1>
                    <div className="navLinks">
                        <Link to="/wasm">WASM</Link>
                        <Link to="/health">Health</Link>
                    </div>
                </div>
                <div className="muted">
                    Catalog: <code>public/actions-catalog.json</code>
                </div>

                {backendStatus ? (
                    <div className="muted" style={{ marginTop: 6 }}>
                        Backend: <code>{backendStatus}</code>
                    </div>
                ) : null}

                <div className="card">
                    <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                        <strong>Status</strong>
                        <span className="muted">
                            {catalog ? `schema_version=${catalog.schema_version}` : 'not loaded'}
                        </span>
                    </div>
                    {catalogError ? (
                        <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>{catalogError}</div>
                    ) : null}
                    {catalog?.executor_component?.digest ? (
                        <div className="muted" style={{ marginTop: 8 }}>
                            digest: <code>{catalog.executor_component.digest}</code>
                        </div>
                    ) : null}
                </div>

                <div className="card">
                    <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                        <strong>Catalog Source</strong>
                        <span className="muted">{catalogLoading ? 'loading…' : 'ready'}</span>
                    </div>

                    <div className="muted" style={{ marginTop: 8 }}>
                        Active:{' '}
                        <code>
                            {catalogSource.kind === 'default'
                                ? '/actions-catalog.json'
                                : catalogSource.kind === 'url'
                                    ? catalogSource.url
                                    : `file:${catalogSource.fileName}`}
                        </code>
                    </div>

                    <div style={{ display: 'flex', gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
                        <button disabled={catalogLoading} onClick={() => reloadCatalog({ kind: 'default' })}>
                            Use default
                        </button>
                        <button
                            disabled={catalogLoading || !catalogUrl.trim()}
                            onClick={() => reloadCatalog({ kind: 'url', url: catalogUrl.trim() })}
                        >
                            Load URL
                        </button>
                        <label style={{ display: 'inline-flex', gap: 8, alignItems: 'center' }}>
                            <span className="muted" style={{ fontSize: 12 }}>
                                Import file
                            </span>
                            <input
                                type="file"
                                accept="application/json,.json"
                                disabled={catalogLoading}
                                onChange={(e) => {
                                    const f = e.target.files && e.target.files.length ? e.target.files[0] : null;
                                    void onPickCatalogFile(f);
                                    // allow re-picking the same file
                                    e.currentTarget.value = '';
                                }}
                            />
                        </label>
                    </div>

                    <input
                        style={{ width: '100%', marginTop: 10 }}
                        placeholder="https://.../actions-catalog.json"
                        value={catalogUrl}
                        onChange={(e) => setCatalogUrl(e.target.value)}
                    />

                    <div className="muted" style={{ marginTop: 8 }}>
                        Tip: for CORS, host the catalog on the same origin or enable CORS headers.
                    </div>
                </div>

                <ActionPalette actions={actions} onPick={addActionNode} />

                <div className="card">
                    <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                        <strong>Quick Nodes</strong>
                        <span className="muted">add</span>
                    </div>
                    <div style={{ display: 'flex', gap: 8, marginTop: 10, flexWrap: 'wrap' }}>
                        <button onClick={() => addQuickNode('start')}>Start</button>
                        <button onClick={() => addQuickNode('wait')}>Wait</button>
                        <button onClick={() => addQuickNode('end')}>End</button>
                    </div>
                </div>

                <div className="card">
                    <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                        <strong>Export</strong>
                        <div style={{ display: 'flex', gap: 8 }}>
                            <span className="muted" style={{ alignSelf: 'center' }}>
                                {errorCount || warningCount ? `${errorCount} error(s), ${warningCount} warning(s)` : 'ok'}
                            </span>
                            <button
                                onClick={async () => {
                                    await navigator.clipboard.writeText(JSON.stringify(exported, null, 2));
                                }}
                            >
                                Copy JSON
                            </button>
                            <button
                                disabled={exportBlockCount > 0}
                                onClick={async () => {
                                    if (exportBlockCount > 0) {
                                        setUiWarning('Export blocked: fix blocking issues first (see Validation).');
                                        return;
                                    }
                                    await navigator.clipboard.writeText(scenarioYaml);
                                }}
                            >
                                Copy scenario.yaml
                            </button>
                        </div>
                    </div>
                    <div className="muted" style={{ marginTop: 6 }}>
                        Exports graph JSON and minimal scenario.yaml.
                    </div>

                    {exportBlockCount > 0 ? (
                        <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>
                            Export is blocked ({exportBlockCount}). See Validation panel.
                        </div>
                    ) : null}

                    <div style={{ marginTop: 10 }}>
                        <label style={{ display: 'flex', gap: 8, alignItems: 'center', fontSize: 12 }}>
                            <input
                                type="checkbox"
                                checked={useScaffoldTemplate}
                                onChange={(e) => setUseScaffoldTemplate(e.target.checked)}
                            />
                            Use scaffold template (YAML/JSON)
                        </label>
                        <div className="muted" style={{ marginTop: 6 }}>
                            When enabled, the exporter merges your template with compiled actions/workflows.
                        </div>
                        {useScaffoldTemplate ? (
                            <>
                                <textarea
                                    className="jsonOutput"
                                    style={{ minHeight: 120 }}
                                    placeholder="Paste a scenario template here (YAML or JSON)."
                                    value={scaffoldText}
                                    onChange={(e) => setScaffoldText(e.target.value)}
                                />
                                {scaffoldError ? (
                                    <div style={{ marginTop: 6, color: '#b91c1c', fontSize: 12 }}>{scaffoldError}</div>
                                ) : null}
                            </>
                        ) : null}
                    </div>

                    <textarea className="jsonOutput" readOnly value={JSON.stringify(exported, null, 2)} />
                    <div className="muted" style={{ marginTop: 10, marginBottom: 6 }}>
                        scenario.yaml (v0)
                    </div>
                    <textarea className="jsonOutput" readOnly value={scenarioYaml} />
                </div>
            </aside>

            <main className="content">
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
            <aside className="rightbar">
                <h1 style={{ fontSize: 16, margin: '0 0 10px 0' }}>Inspector</h1>

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
                            <div className="fieldLabel">action_ref</div>
                            <div className="input small">
                                <code>{String(selectedNodeData?.action_ref ?? '')}</code>
                            </div>

                            <div className="fieldLabel">call</div>
                            <div className="input small">
                                <code>{selectedCall}</code>
                            </div>

                            <div className="fieldLabel">with (JSON object)</div>
                            <textarea
                                className="jsonOutput"
                                style={{ minHeight: 180 }}
                                value={withText}
                                onChange={(e) => setWithText(e.target.value)}
                            />
                            {withError ? (
                                <div style={{ marginTop: 8, color: '#b91c1c', fontSize: 12 }}>
                                    {withError}
                                </div>
                            ) : null}

                            <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
                                <button onClick={applyWithToSelected}>Apply</button>
                                <button
                                    onClick={() => {
                                        if (!selectedCall) return;
                                        const d = findActionDefaults(catalog, selectedCall);
                                        setWithText(JSON.stringify(d, null, 2));
                                        setWithError(null);
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
        </div>
    );
}
