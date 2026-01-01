import { Navigate, Route, Routes } from 'react-router-dom';

import { RequireAuthOutlet } from './RequireAuth';
import { AppLayout } from './layout/AppLayout';
import { BuilderPage } from './pages/BuilderPage';
import { HealthPage } from './pages/HealthPage';
import { HomePage } from './pages/HomePage';
import { LoginPage } from './pages/LoginPage';
import { ConfigPage } from './pages/ConfigPage';
import { WasmListPage } from './pages/WasmListPage';
import { WasmUploadPage } from './pages/WasmUploadPage';

export default function AppRouter() {
    return (
        <Routes>
            <Route element={<AppLayout />}>
                <Route path="/login" element={<LoginPage />} />

                <Route element={<RequireAuthOutlet />}>
                    <Route path="/" element={<HomePage />} />
                    <Route path="/builder" element={<BuilderPage />} />
                    <Route path="/config" element={<ConfigPage />} />
                    <Route path="/health" element={<HealthPage />} />
                    <Route path="/wasm" element={<WasmListPage />} />
                    <Route path="/wasm/upload" element={<WasmUploadPage />} />
                </Route>

                <Route path="*" element={<Navigate to="/" replace />} />
            </Route>
        </Routes>
    );
}
