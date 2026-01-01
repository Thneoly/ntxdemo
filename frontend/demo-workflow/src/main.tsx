import React from 'react';
import ReactDOM from 'react-dom/client';
import { ReactFlowProvider } from 'reactflow';
import 'reactflow/dist/style.css';
import { BrowserRouter } from 'react-router-dom';
import AppRouter from './AppRouter';
import './ui/forms.css';
import './ui/reactflow-nodes.css';
import './styles.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
        <BrowserRouter>
            <ReactFlowProvider>
                <AppRouter />
            </ReactFlowProvider>
        </BrowserRouter>
    </React.StrictMode>
);
