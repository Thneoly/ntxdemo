import type { Edge, Node } from 'reactflow';
import { getNtxNodeType } from '../hooks/useWorkflowEditor';
import { computeReachableFromStart } from './graph';
import { yamlObject } from './yaml';
import { toRecord } from '../utils/toRecord';
import type { ActionsCatalog } from '../types/catalog';

function slugifyIdPart(s: string): string {
    return s
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-+|-+$/g, '')
        .slice(0, 80);
}

function makeUniqueId(base: string, used: Set<string>): string {
    let id = base;
    let i = 2;
    while (used.has(id)) {
        id = `${base}-${i}`;
        i++;
    }
    used.add(id);
    return id;
}

function buildExportNodeIdMap(nodesForExport: Node[]): Map<string, string> {
    // Map React Flow node.id -> exported YAML node.id
    const used = new Set<string>();
    const map = new Map<string, string>();

    // Prefer stable ordering so ids are deterministic.
    const sorted = [...nodesForExport].sort((a, b) => a.id.localeCompare(b.id));

    // First pass: reserve explicit "start" id for start node.
    for (const n of sorted) {
        const t = getNtxNodeType(n);
        if (t === 'start') {
            map.set(n.id, makeUniqueId('start', used));
            break;
        }
    }

    // Second pass: assign readable ids for all other nodes.
    for (const n of sorted) {
        if (map.has(n.id)) continue;
        const t = getNtxNodeType(n);
        const data = toRecord(n.data);

        if (t === 'action') {
            const actionRefRaw = typeof data.action_ref === 'string' && data.action_ref.trim().length ? data.action_ref : 'action';
            const base = `action-${slugifyIdPart(actionRefRaw) || 'action'}`;
            map.set(n.id, makeUniqueId(base, used));
            continue;
        }

        if (t === 'wait') {
            const onObj = toRecord(data.on);
            const evtRaw = typeof onObj.event === 'string' && onObj.event.trim().length ? onObj.event : 'packet-rx';
            const base = `wait-${slugifyIdPart(evtRaw) || 'wait'}`;
            map.set(n.id, makeUniqueId(base, used));
            continue;
        }

        if (t === 'end') {
            map.set(n.id, makeUniqueId('end', used));
            continue;
        }

        // Fallback for unknown/malformed nodes.
        map.set(n.id, makeUniqueId('node', used));
    }

    return map;
}

function safeJsonParseObject(input: string): Record<string, unknown> {
    try {
        const v: unknown = JSON.parse(input);
        if (v && typeof v === 'object' && !Array.isArray(v)) return v as Record<string, unknown>;
        return {};
    } catch {
        return {};
    }
}

function findCatalogDefaults(catalog: ActionsCatalog | undefined, call: string): Record<string, unknown> {
    if (!catalog) return {};
    const entry = catalog.actions.find((a) => a.summary.id === call);
    const spec = entry?.spec as unknown as Record<string, unknown> | undefined;
    const defaultsJson =
        (typeof entry?.spec?.default_params_json === 'string' ? entry?.spec?.default_params_json : undefined) ??
        (spec && typeof spec['default-params-json'] === 'string' ? (spec['default-params-json'] as string) : undefined) ??
        (typeof entry?.spec?.defaults_json === 'string' ? entry?.spec?.defaults_json : undefined) ??
        undefined;
    if (!defaultsJson) return {};
    return safeJsonParseObject(defaultsJson);
}

function normalizeActionWithParams(withObj: Record<string, unknown>): Record<string, unknown> {
    // Runtime contract (ntx-action-sdk): payload is required as one of
    // payload / payload_hex / payload_bytes.
    // The demo catalog historically used payload_utf8; map it to payload.
    const out: Record<string, unknown> = { ...withObj };

    if (out.payload === undefined && typeof out.payload_utf8 === 'string') {
        out.payload = out.payload_utf8;
    }
    // Avoid emitting legacy UI/catalog key.
    delete out.payload_utf8;

    return out;
}

export type ExportOptions = {
    // Optional: used for action.with defaults. Frontend gets this from /actions-catalog.json.
    catalog?: ActionsCatalog;
    // If true, include a minimal UDP workbook/load scaffold (demo convenience).
    // For a general workflow builder, callers can provide their own scaffold or turn this off.
    includeDemoScaffold?: boolean;
    // Optional: scenario scaffold template (YAML/JSON imported by user). This should be the parsed
    // object form of a scenario.
    scaffold?: Record<string, unknown>;
};

function deepMerge(a: unknown, b: unknown): unknown {
    // Merges b into a.
    // - objects: deep merge
    // - arrays/primitives: b wins
    if (a && typeof a === 'object' && !Array.isArray(a) && b && typeof b === 'object' && !Array.isArray(b)) {
        const out: Record<string, unknown> = { ...(a as Record<string, unknown>) };
        for (const [k, bv] of Object.entries(b as Record<string, unknown>)) {
            out[k] = deepMerge(out[k], bv);
        }
        return out;
    }
    return b === undefined ? a : b;
}

export function buildScenarioYaml(args: { workflowName: string; nodes: Node[]; edges: Edge[]; options?: ExportOptions }): string {
    const { workflowName, nodes, edges, options } = args;
    const catalog = options?.catalog;
    const includeDemoScaffold = options?.includeDemoScaffold ?? true;
    const scaffold = options?.scaffold;

    const reachable = computeReachableFromStart(nodes, edges);
    const nodesForExport = reachable.size ? nodes.filter((n) => reachable.has(n.id)) : nodes;
    const edgesForExport = reachable.size
        ? edges.filter((e) => reachable.has(e.source) && reachable.has(e.target))
        : edges;

    const nodeIdMap = buildExportNodeIdMap(nodesForExport);

    // FRONTEND.md mapping:
    // - Node.data.action_ref references actions.actions[*].id
    // - Node.data.call is the executor action id
    // - Node.data.with is the YAML 'with' map
    const actionDefs = Array.from(
        new Map(
            nodesForExport
                .map((n) => {
                    const data = toRecord(n.data);
                    const actionRef = typeof data.action_ref === 'string' ? data.action_ref : undefined;
                    const call = typeof data.call === 'string' ? data.call : undefined;
                    const withObj = toRecord(data.with);
                    if (!actionRef || !call) return null;
                    const defaults = findCatalogDefaults(catalog, call);
                    // defaults first, then user overrides.
                    const mergedWith = normalizeActionWithParams({
                        ...defaults,
                        ...withObj,
                    });
                    return [actionRef, { id: actionRef, call, with: mergedWith }] as const;
                })
                .filter((x): x is readonly [string, { id: string; call: string; with: Record<string, unknown> }] => Boolean(x))
        ).values()
    ).sort((a, b) => a.id.localeCompare(b.id));

    const outgoingBySource = new Map<string, Array<{ to: string; label?: string }>>();
    for (const e of edgesForExport) {
        const src = nodeIdMap.get(e.source) ?? e.source;
        const dst = nodeIdMap.get(e.target) ?? e.target;
        const arr = outgoingBySource.get(src) ?? [];
        // For now, edge label is optional (React Flow edge label or edge.data.label).
        // Many graphs don't set any label, and that's OK.
        const edgeData = toRecord((e as unknown as { data?: unknown }).data);
        const labelFromData = typeof edgeData.label === 'string' ? edgeData.label : undefined;
        const labelFromEdge = typeof (e as unknown as { label?: unknown }).label === 'string' ? ((e as unknown as { label?: string }).label as string) : undefined;
        arr.push({ to: dst, label: labelFromData ?? labelFromEdge ?? undefined });
        outgoingBySource.set(src, arr);
    }

    const incomingByTarget = new Map<string, string[]>();
    for (const e of edgesForExport) {
        const src = nodeIdMap.get(e.source) ?? e.source;
        const dst = nodeIdMap.get(e.target) ?? e.target;
        const arr = incomingByTarget.get(dst) ?? [];
        arr.push(src);
        incomingByTarget.set(dst, arr);
    }

    const startNode = nodesForExport.find((n) => getNtxNodeType(n) === 'start') ?? null;
    const startNodeOriginalId = startNode?.id ?? null;
    const startNodeExportId = startNodeOriginalId ? nodeIdMap.get(startNodeOriginalId) ?? 'start' : 'start';

    const wfNodes = nodesForExport.map((n) => {
        const data = toRecord(n.data);
        const actionRef = typeof data.action_ref === 'string' ? data.action_ref : undefined;
        const exportId = nodeIdMap.get(n.id) ?? n.id;
        const out = outgoingBySource.get(exportId) ?? [];

        const ntxType = getNtxNodeType(n);
        if (ntxType === 'wait') {
            const onObj = toRecord(data.on);
            const matchObj = toRecord(onObj.match);

            // If user didn't specify match.action_id, infer it from the *incoming* action node.
            // This is important for a general builder because wait nodes are often placed after an action.
            const incomingExportIds = incomingByTarget.get(exportId) ?? [];
            const upstreamAction = incomingExportIds
                .map((exportUpId) => {
                    // reverse lookup: export id -> original node
                    const orig = nodesForExport.find((x) => (nodeIdMap.get(x.id) ?? x.id) === exportUpId) ?? null;
                    return orig;
                })
                .find((x) => x && getNtxNodeType(x) === 'action');
            const upstreamData = upstreamAction ? toRecord(upstreamAction.data) : {};
            const upstreamActionRef = typeof upstreamData.action_ref === 'string' ? (upstreamData.action_ref as string) : null;

            const mergedMatch: Record<string, unknown> = {
                ...(upstreamActionRef ? { action_id: upstreamActionRef } : {}),
                ...matchObj,
            };
            return {
                id: exportId,
                type: 'wait',
                on: {
                    event: typeof onObj.event === 'string' && onObj.event.length ? onObj.event : 'packet-rx',
                    match: Object.keys(mergedMatch).length ? mergedMatch : undefined,
                },
                ...(out.length
                    ? {
                        edges: out.map((o) => ({
                            to: o.to,
                            ...(o.label ? { label: o.label } : {}),
                        })),
                    }
                    : {}),
            };
        }

        if (ntxType === 'start') {
            // Generic builder: keep `type: start` in the edit graph.
            // Export: prefer `type: start` if it has no clear action semantics.
            // If start directly connects to exactly one action node, we can choose to export
            // it as an `action` task (udp-echo-minimal style) for better runtime compatibility.
            const nextId = out.length ? out[0].to : null;
            const next = nextId
                ? nodesForExport.find((x) => (nodeIdMap.get(x.id) ?? x.id) === nextId) ?? null
                : null;
            const nextData = next ? toRecord(next.data) : {};
            const nextActionRef = next && getNtxNodeType(next) === 'action' && typeof nextData.action_ref === 'string' ? (nextData.action_ref as string) : null;

            if (nextActionRef) {
                // Export as action-like start (demo compatible).
                return {
                    id: startNodeExportId,
                    type: 'action',
                    priority: 10,
                    action: nextActionRef,
                    ...(out.length
                        ? {
                            edges: [
                                {
                                    to: nextId,
                                    label: out[0]?.label ?? 'sent',
                                },
                            ],
                        }
                        : {}),
                };
            }

            return {
                id: exportId,
                type: 'start',
                ...(out.length
                    ? {
                        edges: out.map((o) => ({
                            to: o.to,
                            ...(o.label ? { label: o.label } : {}),
                        })),
                    }
                    : {}),
            };
        }

        if (ntxType === 'end') {
            return {
                id: exportId,
                type: 'end',
            };
        }

        return {
            id: exportId,
            type: actionRef ? 'action' : 'end',
            priority: 10,
            ...(actionRef ? { action: actionRef } : {}),
            ...(out.length
                ? {
                    edges: out.map((o) => ({
                        to: o.to,
                        ...(o.label ? { label: o.label } : {}),
                    })),
                }
                : {}),
        };
    });

    // Scenario scaffold:
    // - For a general workflow builder, we keep a small default but allow callers to
    //   disable the demo-specific workbook/load blocks.
    const compiled: Record<string, unknown> = {
        version: 'v1',
        name: workflowName,
        actions: {
            actions: actionDefs,
        },
        workflows: {
            nodes: wfNodes,
        },
    };

    const demoScaffold: Record<string, unknown> = includeDemoScaffold
        ? {
            workbook: {
                resources: [
                    {
                        id: 'udp-target',
                        type: 'udp-endpoint',
                        properties: {
                            peer_ip: '10.0.0.2',
                            peer_port: 7,
                            peer_mac: '02:00:00:00:00:a2',
                            pool: 'default',
                        },
                    },
                ],
            },
            load: {
                ramp_up: {
                    phases: [
                        { at_second: 1, spawn_users: 1 },
                        { at_second: 5, spawn_users: 1 },
                    ],
                },
                user_lifetime: {
                    mode: 'once',
                    max_concurrency: 1,
                },
            },
            user_resources: {
                ip_binding: {
                    enabled: true,
                    pool_id: 'default',
                },
            },
        }
        : {};

    // Merge order (later wins): demoScaffold <- scaffoldTemplate <- compiled
    const merged = deepMerge(deepMerge(demoScaffold, scaffold ?? {}), compiled) as Record<string, unknown>;
    const scenario: Record<string, unknown> = merged;

    return yamlObject(scenario, 0) + '\n';
}
