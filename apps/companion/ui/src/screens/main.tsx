// The screens page uses Preact (architecture 7). It exists only while a screen is open:
// the window is created on demand and destroyed on close (A7).
// Slice V1-1 provides Settings > Pet (wireframe W3). Other settings arrive with their
// slices.
import { render } from 'preact';
import { PetSettings } from './PetSettings';

function App() {
  return (
    <div class="shell">
      <header class="top">
        <h1>Settings</h1>
      </header>
      <PetSettings />
    </div>
  );
}

const root = document.getElementById('app');
if (root) {
  render(<App />, root);
}
