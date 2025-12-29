import type { ActionSummary } from '../types/catalog';

export function ActionPalette(props: {
    actions: ActionSummary[];
    onPick: (action: ActionSummary) => void;
}) {
    const { actions, onPick } = props;

    return (
        <div className="card">
            <div className="toolbar" style={{ justifyContent: 'space-between' }}>
                <strong>Actions</strong>
                <span className="muted">click to add</span>
            </div>

            <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 10 }}>
                {actions.map((a) => (
                    <div
                        key={a.id}
                        className="actionItem"
                        role="button"
                        tabIndex={0}
                        onClick={() => onPick(a)}
                        onKeyDown={(e) => {
                            if (e.key === 'Enter' || e.key === ' ') onPick(a);
                        }}
                    >
                        <div className="actionId">{a.id}</div>
                        <div className="actionDesc">{a.description ?? a.title ?? ''}</div>
                    </div>
                ))}
            </div>
        </div>
    );
}
