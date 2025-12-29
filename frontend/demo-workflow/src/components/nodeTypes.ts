import type { NodeTypes } from 'reactflow';
import { ActionNode, EndNode, StartNode, WaitNode } from './NtxNodes';

export const nodeTypes: NodeTypes = {
    start: StartNode,
    action: ActionNode,
    wait: WaitNode,
    end: EndNode,
};
