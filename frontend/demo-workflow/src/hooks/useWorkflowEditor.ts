import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
    addEdge,
    type Connection,
    type Edge,
    type Node,
    useEdgesState,
    useNodesState,
} from 'reactflow';

import { toRecord } from '../utils/toRecord';

export type NtxNodeType = 'start' | 'action' | 'wait' | 'end';

export type ValidationIssue = {
    level: 'warning' | 'error';
    message: string;
    // Optional: tie an issue to a specific node so the UI can jump to it.
    nodeId?: string;
};

export function getNtxNodeType(n: Node): NtxNodeType {
    const data = toRecord(n.data);
    const t = data.ntx_node_type;
    if (t === 'start' || t === 'action' || t === 'wait' || t === 'end') return t;
    if (typeof data.action_ref === 'string') return 'action';
    return 'end';
}

export type UseWorkflowEditorArgs = {
    onInvalidConnect?: (message: string) => void;
};

/**
 * Workflow editor domain state (nodes/edges) + guarded mutations.
 * Keeps callbacks stable and provides refs for hotkeys.
 */
export function useWorkflowEditor(args: UseWorkflowEditorArgs = {}) {
    const [nodes, setNodes, onNodesChange] = useNodesState<Node[]>([]);
    const [edges, setEdges, onEdgesChange] = useEdgesState<Edge[]>([]);

    const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
    const [uiWarning, setUiWarning] = useState<string | null>(null);

    const selectedNodeIdRef = useRef<string | null>(null);
    useEffect(() => {
        selectedNodeIdRef.current = selectedNodeId;
    }, [selectedNodeId]);

    const selectedNode = useMemo(
        () => (selectedNodeId ? nodes.find((n: Node) => n.id === selectedNodeId) ?? null : null),
        [nodes, selectedNodeId]
    );

    const deleteNodeById = useCallback((id: string) => {
        setNodes((ns: Node[]) => ns.filter((n) => n.id !== id));
        setEdges((es: Edge[]) => es.filter((e) => e.source !== id && e.target !== id));
        setSelectedNodeId((cur: string | null) => (cur === id ? null : cur));
    }, []);

    const deleteNodeByIdRef = useRef<(id: string) => void>(() => undefined);
    useEffect(() => {
        deleteNodeByIdRef.current = deleteNodeById;
    }, [deleteNodeById]);

    const deleteSelectedNode = useCallback(() => {
        const id = selectedNodeIdRef.current;
        if (!id) return;
        deleteNodeByIdRef.current(id);
    }, []);

    const setNodeAsStart = useCallback((id: string) => {
        setNodes((ns: Node[]) =>
            ns.map((n) => {
                const data = toRecord(n.data);
                if (n.id === id) {
                    return {
                        ...n,
                        type: 'start',
                        data: {
                            ...data,
                            ntx_node_type: 'start',
                            label: 'start',
                        },
                    };
                }
                if (getNtxNodeType(n) === 'start') {
                    return {
                        ...n,
                        type: 'end',
                        data: {
                            ...data,
                            ntx_node_type: 'end',
                            label: 'end',
                        },
                    };
                }
                return n;
            })
        );
    }, []);

    const onConnect = useCallback(
        (connection: Connection) => {
            const sourceId = connection.source;
            if (sourceId) {
                const srcNode = nodes.find((n: Node) => n.id === sourceId);
                if (srcNode && getNtxNodeType(srcNode) === 'end') {
                    const msg = 'End nodes cannot have outgoing edges.';
                    setUiWarning(msg);
                    args.onInvalidConnect?.(msg);
                    return;
                }
            }
            setUiWarning(null);
            setEdges((eds: Edge[]) => addEdge({ ...connection, id: `e-${crypto.randomUUID()}` }, eds));
        },
        [args, nodes]
    );

    // Stable helper to inject node-local actions for custom node UIs.
    const withNodeActions = useCallback(
        (nodeId: string, data: Record<string, unknown>) => {
            return {
                ...data,
                _ntx: {
                    deleteSelf: () => deleteNodeById(nodeId),
                    setAsStart: () => setNodeAsStart(nodeId),
                },
            };
        },
        [deleteNodeById, setNodeAsStart]
    );

    // Hotkey wiring (stable; consult refs)
    useEffect(() => {
        const handler = (e: KeyboardEvent) => {
            if (e.key !== 'Delete' && e.key !== 'Backspace') return;
            const el = e.target as HTMLElement | null;
            const tag = el?.tagName?.toLowerCase();
            const isEditable =
                tag === 'input' ||
                tag === 'textarea' ||
                tag === 'select' ||
                (el ? el.isContentEditable : false);
            if (isEditable) return;

            const id = selectedNodeIdRef.current;
            if (!id) return;

            e.preventDefault();
            deleteNodeByIdRef.current(id);
        };

        window.addEventListener('keydown', handler);
        return () => window.removeEventListener('keydown', handler);
    }, []);

    return {
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

        deleteNodeById,
        deleteSelectedNode,
        setNodeAsStart,

        withNodeActions,
    };
}
