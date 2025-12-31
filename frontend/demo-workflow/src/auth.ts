const AUTH_KEY = 'ntx.demo.auth.v1';

type StoredAuth = {
    user: string;
    loginAt: string;
};

export function getAuthUser(): string | null {
    try {
        const raw = localStorage.getItem(AUTH_KEY);
        if (!raw) return null;
        const parsed = JSON.parse(raw) as unknown;
        if (!parsed || typeof parsed !== 'object') return null;
        const rec = parsed as Partial<StoredAuth>;
        if (typeof rec.user !== 'string' || !rec.user.trim()) return null;
        return rec.user;
    } catch {
        return null;
    }
}

export function isLoggedIn(): boolean {
    return getAuthUser() !== null;
}

export function login(user: string): void {
    const trimmed = user.trim();
    if (!trimmed) return;
    const payload: StoredAuth = { user: trimmed, loginAt: new Date().toISOString() };
    localStorage.setItem(AUTH_KEY, JSON.stringify(payload));
}

export function logout(): void {
    localStorage.removeItem(AUTH_KEY);
}
