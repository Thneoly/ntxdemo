import { Link } from 'react-router-dom';

import { getAuthUser } from '../auth';

export function HomePage() {
    const user = getAuthUser();

    return (
        <div style={{ padding: 16, maxWidth: 720 }}>
            <h1 style={{ margin: 0, fontSize: 18 }}>Home</h1>
            <div style={{ marginTop: 8, color: '#666' }}>{user ? `Logged in as ${user}` : null}</div>

            <div style={{ marginTop: 14, display: 'flex', gap: 12, flexWrap: 'wrap' }}>
                <Link to="/builder">Builder</Link>
                <Link to="/wasm">WASM</Link>
                <Link to="/health">Health</Link>
            </div>
        </div>
    );
}
