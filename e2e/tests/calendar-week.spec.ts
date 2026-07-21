/**
 * CalendarWeek — the dashboard's news block on the HOME window.
 *
 * The at-a-glance view: the next high-impact release with a countdown, and
 * seven day-columns beside it. The detailed day-grouped list still exists
 * below as "All releases", collapsed by default so the same events are not
 * rendered twice at full weight.
 */
import { test, expect } from '../helpers/app-fixture';

const now = Math.floor(Date.now() / 1000);
const ev = (hoursAhead: number, currency: string, name: string, impact: string) => {
  const t = now + Math.round(hoursAhead * 3600);
  const d = new Date(t * 1000);
  return {
    date: d.toISOString().slice(0, 10),
    time: d.toISOString().slice(11, 16),
    time_unix: t,
    currency,
    event: name,
    impact,
    actual: '',
    forecast: '0.2%',
    previous: '0.1%',
  };
};

const EVENTS = [
  ev(2, 'GBP', 'Claimant Count Change', 'high'),
  ev(4, 'USD', 'Core CPI m/m', 'high'),
  ev(6, 'USD', 'Retail Sales m/m', 'medium'),
  ev(30, 'EUR', 'ECB Press Conference', 'high'),
  ev(100, 'JPY', 'BOJ Policy Rate', 'high'),
];

test.describe('Calendar week', () => {
  test('leads with the next high-impact release and a countdown', async ({ appPage }) => {
    await appPage.mockTauriCommand('get_economic_calendar', EVENTS);
    await appPage.goto('local');

    const next = appPage.page.getByTestId('calendar-week-next-high');
    await expect(next).toContainText('Claimant Count Change');
    await expect(next).toContainText('GBP');
    await expect(next).toContainText(/in \d+h/);
  });

  test('skips medium impact when picking the next-up release', async ({ appPage }) => {
    // A medium print sooner must not displace the high-impact one — "what's
    // the next big thing" is the question being answered.
    await appPage.mockTauriCommand('get_economic_calendar', [
      ev(1, 'CAD', 'Wholesale Sales m/m', 'medium'),
      ev(3, 'USD', 'Core CPI m/m', 'high'),
    ]);
    await appPage.goto('local');

    const next = appPage.page.getByTestId('calendar-week-next-high');
    await expect(next).toContainText('Core CPI m/m');
    await expect(next).not.toContainText('Wholesale');
  });

  test('renders seven day columns including quiet ones', async ({ appPage }) => {
    await appPage.mockTauriCommand('get_economic_calendar', EVENTS);
    await appPage.goto('local');

    // An empty column is information: it says that day is quiet.
    await expect(appPage.page.getByTestId('calendar-week-day')).toHaveCount(7);
    await expect(appPage.page.getByTestId('calendar-week-grid')).toContainText('Today');
    await expect(appPage.page.getByTestId('calendar-week-grid')).toContainText('Tomorrow');
  });

  test('shows full event names in the columns, not truncated fragments', async ({ appPage }) => {
    await appPage.mockTauriCommand('get_economic_calendar', EVENTS);
    await appPage.goto('local');

    await expect(
      appPage.page.getByTestId('calendar-week-event').filter({ hasText: 'Claimant Count Change' })
    ).toHaveCount(1);
  });

  test('degrades to a plain message when the store is empty', async ({ appPage }) => {
    await appPage.mockTauriCommand('get_economic_calendar', []);
    await appPage.goto('local');

    await expect(appPage.page.getByTestId('calendar-week')).toContainText(
      'No high-impact releases'
    );
    await expect(appPage.page.getByTestId('calendar-week-day')).toHaveCount(7);
  });

  test('the detailed list is present but collapsed by default', async ({ appPage }) => {
    await appPage.mockTauriCommand('get_economic_calendar', EVENTS);
    await appPage.goto('local');

    await expect(appPage.page.getByRole('heading', { name: 'All releases' })).toBeVisible();
    // Collapsed: its event rows are not rendered until opened.
    await expect(appPage.page.getByTestId('calendar-event-row')).toHaveCount(0);
  });
});
