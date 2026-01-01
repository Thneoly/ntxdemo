import { Link, NavLink, Outlet, useLocation, useNavigate } from 'react-router-dom';

import { getAuthUser, isLoggedIn, logout } from '../auth';

export function AppLayout() {
    const nav = useNavigate();
    const location = useLocation();

    const loggedIn = isLoggedIn();
    const user = getAuthUser();

    function onLogout() {
        logout();
        nav('/login', { replace: true, state: { from: location.pathname + location.search } });
    }

    return (
        <div className="appShell">
            <header className="appHeader">
                <div className="appHeaderInner">
                    <div className="appHeaderLeft">
                        <Link to={loggedIn ? '/' : '/login'} className="appBrand">
                            Ntx
                        </Link>
                        {loggedIn ? (
                            <nav className="navLinks">
                                <NavLink to="/" end>
                                    Home
                                </NavLink>
                                <NavLink to="/wasm">WASM</NavLink>
                                <NavLink to="/config">Config</NavLink>
                                <NavLink to="/builder">Builder</NavLink>
                                <NavLink to="/health">Health</NavLink>
                            </nav>
                        ) : null}
                    </div>

                    <div className="appHeaderRight">
                        {loggedIn ? (
                            <>
                                <span className="muted appHeaderUser">
                                    {user ?? '—'}
                                </span>
                                <button type="button" onClick={onLogout}>
                                    Logout
                                </button>
                            </>
                        ) : (
                            <Link to="/login" className="buttonLike">
                                Login
                            </Link>
                        )}
                    </div>
                </div>
            </header>

            <main className="appMain">
                <Outlet />
            </main>

            <footer className="appFooter">
                <div className="appFooterInner">
                    <div>Ntx demo-workflow</div>
                    <div className="muted">{loggedIn && user ? `Signed in as ${user}` : 'Not signed in'}</div>
                </div>
            </footer>
        </div>
    );
}
