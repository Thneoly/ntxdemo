import { getAuthUser } from '../auth';

export function HomePage() {
    const user = getAuthUser();

    return (
        <div className="page">
            <h1 className="pageTitleBlock">
                Home
            </h1>

            <div className="card" style={{ maxWidth: 520 }}>
                <div style={{ fontWeight: 600 }}>Session</div>
                <div className="muted" style={{ marginTop: 6 }}>
                    {user ? `Logged in as ${user}` : '—'}
                </div>
            </div>
        </div>
    );
}
