import { Link } from 'react-router-dom';

import { getAuthUser } from '../auth';

export function HomePage() {
    const user = getAuthUser();

    return (
        <div className="page">
            <div className="pageHeader">
                <h1 className="pageTitle">Home</h1>
                <div className="pageActions">
                    <Link to="/builder">Builder</Link>
                    <Link to="/config">Config</Link>
                    <Link to="/wasm">WASM</Link>
                    <Link to="/health">Health</Link>
                </div>
            </div>

            <div className="card" style={{ maxWidth: 520 }}>
                <div style={{ fontWeight: 600 }}>Session</div>
                <div className="muted" style={{ marginTop: 6 }}>
                    {user ? `Logged in as ${user}` : '—'}
                </div>
            </div>
        </div>
    );
}
