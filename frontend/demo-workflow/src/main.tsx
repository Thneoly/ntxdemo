import React from 'react';
import ReactDOM from 'react-dom/client';
import { ReactFlowProvider } from 'reactflow';
import { BrowserRouter } from 'react-router-dom';
import AppRouter from './AppRouter';
import './styles.css';
import './ui/primitives.css';
import './ui/forms.css';
import 'reactflow/dist/style.css';
import './ui/reactflow-nodes.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
        <BrowserRouter>
            <ReactFlowProvider>
                <AppRouter />
            </ReactFlowProvider>
        </BrowserRouter>
    </React.StrictMode>
);
