import { memo } from 'react';
import { Handle, Position, type NodeProps } from 'reactflow';

function NodeShell(props: {
    title: string;
    subtitle?: string;
    badge?: string;
    variant: 'start' | 'action' | 'wait' | 'end';
    highlight?: boolean;
    children?: React.ReactNode;
}) {
    const { title, subtitle, badge, variant, highlight, children } = props;

    return (
        <div className={`ntxNode ntxNode--${variant}${highlight ? ' ntxNode--highlight' : ''}`}>
            <div className="ntxNodeHeader">
                <div className="ntxNodeTitleRow">
                    <div className="ntxNodeTitle">{title}</div>
                    {badge ? <div className="ntxNodeBadge">{badge}</div> : null}
                </div>
                {subtitle ? <div className="ntxNodeSubtitle">{subtitle}</div> : null}
            </div>
            {children ? <div className="ntxNodeBody">{children}</div> : null}
        </div>
    );
}

export const StartNode = memo(function StartNode(_props: NodeProps) {
    const data = (_props.data ?? {}) as Record<string, unknown>;
    const highlight = data._highlight === true;
    const ntx = (data._ntx && typeof data._ntx === 'object' && !Array.isArray(data._ntx) ? (data._ntx as Record<string, unknown>) : {}) as Record<
        string,
        unknown
    >;
    const deleteSelf = typeof ntx.deleteSelf === 'function' ? (ntx.deleteSelf as () => void) : null;
    return (
        <>
            <NodeShell title="Start" subtitle="entry" variant="start" badge="start" highlight={highlight}>
                <div className="ntxNodeActions">
                    {deleteSelf ? (
                        <button className="ntxNodeBtn" onClick={(e) => (e.stopPropagation(), deleteSelf())}>
                            Delete
                        </button>
                    ) : null}
                </div>
            </NodeShell>
            <Handle type="source" position={Position.Right} />
        </>
    );
});

export const EndNode = memo(function EndNode(_props: NodeProps) {
    const data = (_props.data ?? {}) as Record<string, unknown>;
    const highlight = data._highlight === true;
    const ntx = (data._ntx && typeof data._ntx === 'object' && !Array.isArray(data._ntx) ? (data._ntx as Record<string, unknown>) : {}) as Record<
        string,
        unknown
    >;
    const deleteSelf = typeof ntx.deleteSelf === 'function' ? (ntx.deleteSelf as () => void) : null;
    const setAsStart = typeof ntx.setAsStart === 'function' ? (ntx.setAsStart as () => void) : null;
    return (
        <>
            <Handle type="target" position={Position.Left} />
            <NodeShell title="End" subtitle="exit" variant="end" badge="end" highlight={highlight}>
                <div className="ntxNodeActions">
                    {setAsStart ? (
                        <button className="ntxNodeBtn" onClick={(e) => (e.stopPropagation(), setAsStart())}>
                            Set Start
                        </button>
                    ) : null}
                    {deleteSelf ? (
                        <button className="ntxNodeBtn ntxNodeBtn--danger" onClick={(e) => (e.stopPropagation(), deleteSelf())}>
                            Delete
                        </button>
                    ) : null}
                </div>
            </NodeShell>
        </>
    );
});

export const ActionNode = memo(function ActionNode(props: NodeProps) {
    const data = (props.data ?? {}) as Record<string, unknown>;
    const highlight = data._highlight === true;
    const actionRef = typeof data.action_ref === 'string' ? data.action_ref : '';
    const call = typeof data.call === 'string' ? data.call : '';
    const ntx = (data._ntx && typeof data._ntx === 'object' && !Array.isArray(data._ntx) ? (data._ntx as Record<string, unknown>) : {}) as Record<
        string,
        unknown
    >;
    const deleteSelf = typeof ntx.deleteSelf === 'function' ? (ntx.deleteSelf as () => void) : null;
    const setAsStart = typeof ntx.setAsStart === 'function' ? (ntx.setAsStart as () => void) : null;

    return (
        <>
            <Handle type="target" position={Position.Left} />
            <NodeShell title={actionRef || 'Action'} subtitle={call ? `call: ${call}` : undefined} variant="action" badge="action" highlight={highlight}>
                <div className="ntxNodeMono">{call}</div>
                <div className="ntxNodeActions">
                    {setAsStart ? (
                        <button className="ntxNodeBtn" onClick={(e) => (e.stopPropagation(), setAsStart())}>
                            Set Start
                        </button>
                    ) : null}
                    {deleteSelf ? (
                        <button className="ntxNodeBtn ntxNodeBtn--danger" onClick={(e) => (e.stopPropagation(), deleteSelf())}>
                            Delete
                        </button>
                    ) : null}
                </div>
            </NodeShell>
            <Handle type="source" position={Position.Right} />
        </>
    );
});

export const WaitNode = memo(function WaitNode(props: NodeProps) {
    const data = (props.data ?? {}) as Record<string, unknown>;
    const highlight = data._highlight === true;
    const onObj = (data.on && typeof data.on === 'object' && !Array.isArray(data.on) ? (data.on as Record<string, unknown>) : {}) as Record<
        string,
        unknown
    >;
    const evt = typeof onObj.event === 'string' ? onObj.event : '';
    const ntx = (data._ntx && typeof data._ntx === 'object' && !Array.isArray(data._ntx) ? (data._ntx as Record<string, unknown>) : {}) as Record<
        string,
        unknown
    >;
    const deleteSelf = typeof ntx.deleteSelf === 'function' ? (ntx.deleteSelf as () => void) : null;
    const setAsStart = typeof ntx.setAsStart === 'function' ? (ntx.setAsStart as () => void) : null;

    return (
        <>
            <Handle type="target" position={Position.Left} />
            <NodeShell title="Wait" subtitle={evt ? `on: ${evt}` : 'on: (unset)'} variant="wait" badge="wait" highlight={highlight}>
                <div className="ntxNodeActions">
                    {setAsStart ? (
                        <button className="ntxNodeBtn" onClick={(e) => (e.stopPropagation(), setAsStart())}>
                            Set Start
                        </button>
                    ) : null}
                    {deleteSelf ? (
                        <button className="ntxNodeBtn ntxNodeBtn--danger" onClick={(e) => (e.stopPropagation(), deleteSelf())}>
                            Delete
                        </button>
                    ) : null}
                </div>
            </NodeShell>
            <Handle type="source" position={Position.Right} />
        </>
    );
});
