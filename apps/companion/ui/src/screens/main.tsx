// The screens page uses Preact (architecture 7). It exists only while a screen is open.
// V1-0 skeleton: setup, settings, sharing and reviews are slices V1-1 to V1-4.
import { render } from 'preact';
import { SLICE, scaffoldNotice } from '../shared/build';

function App() {
  return (
    <main>
      <h1>HealthierYou</h1>
      <p>{scaffoldNotice('Screens')}</p>
      <p>
        Slice {SLICE} provides the workspace, the shared core schema, the local store and its
        migrations, and the clock and time-zone rules. Nothing is shared and no server is used.
      </p>
    </main>
  );
}

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}
