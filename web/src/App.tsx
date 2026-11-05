import { Routes, Route } from 'react-router-dom'
import { Layout } from './components/Layout'
import { ArchiveLayout } from './components/ArchiveLayout'
import { Welcome } from './pages/Welcome'
import { DayDetail } from './pages/DayDetail'
import { SessionDetail } from './pages/SessionDetail'
import { Settings } from './pages/Settings'
import { Insights } from './pages/Insights'
import { Board } from './pages/Board'

export default function App() {
  return (
    <Routes>
      <Route path="/" element={<Layout />}>
        {/* Archive routes with dual-panel layout */}
        <Route element={<ArchiveLayout />}>
          <Route index element={<Welcome />} />
          <Route path="day/:date" element={<DayDetail />} />
          <Route path="day/:date/session/:name" element={<SessionDetail />} />
        </Route>

        {/* Other routes without archive tree */}
        <Route path="board" element={<Board />} />
        <Route path="settings" element={<Settings />} />
        <Route path="insights" element={<Insights />} />
      </Route>
    </Routes>
  )
}
