#!/usr/bin/env npx tsx
/**
 * ForexFactory Thread Scraper
 *
 * Scrapes posts from a ForexFactory forum thread and saves them as markdown.
 * Uses Puppeteer to bypass Cloudflare protection. Runs in headful (visible)
 * mode by default for reliable Cloudflare handling.
 *
 * Usage:
 *   npx tsx scripts/scrape-forexfactory.ts <thread-url> [--pages <n>] [--output <file>] [--headless]
 *
 * Examples:
 *   npx tsx scripts/scrape-forexfactory.ts https://www.forexfactory.com/thread/123456-some-thread
 *   npx tsx scripts/scrape-forexfactory.ts https://www.forexfactory.com/thread/123456-some-thread --pages 5
 *   npx tsx scripts/scrape-forexfactory.ts https://www.forexfactory.com/thread/123456-some-thread --headless
 */

import puppeteer from 'puppeteer-extra';
import StealthPlugin from 'puppeteer-extra-plugin-stealth';
import { type Page } from 'puppeteer';
import { writeFileSync } from 'fs';
import { join } from 'path';

puppeteer.use(StealthPlugin());

interface Post {
  author: string;
  date: string;
  postNumber: string;
  content: string;
}

function parseArgs() {
  const args = process.argv.slice(2);
  if (args.length === 0 || args[0].startsWith('--')) {
    console.error(
      'Usage: npx tsx scripts/scrape-forexfactory.ts <thread-url> [--pages <n>] [--output <file>] [--headless]'
    );
    process.exit(1);
  }

  const url = args[0];
  if (!url.includes('forexfactory.com/thread')) {
    console.error('Error: URL must be a ForexFactory thread URL');
    process.exit(1);
  }

  let maxPages = 10;
  let output = '';
  let headless = false;

  for (let i = 1; i < args.length; i++) {
    if (args[i] === '--pages' && args[i + 1]) {
      maxPages = parseInt(args[i + 1], 10);
      i++;
    } else if (args[i] === '--output' && args[i + 1]) {
      output = args[i + 1];
      i++;
    } else if (args[i] === '--headless') {
      headless = true;
    }
  }

  return { url, maxPages, output, headless };
}

async function waitForContent(page: Page): Promise<void> {
  // Cloudflare may show a challenge — poll until it resolves or timeout
  const maxWait = 90000;
  const start = Date.now();
  while (Date.now() - start < maxWait) {
    const title = await page.title();
    if (!title.includes('Just a moment')) {
      try {
        await page.waitForSelector('.threadpost-content__message', { timeout: 10000 });
        return;
      } catch {
        // Page loaded but no posts yet — might be loading
      }
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
  throw new Error('Timed out waiting for page content.');
}

async function extractPosts(page: Page): Promise<Post[]> {
  return page.evaluate(() => {
    const posts: Post[] = [];
    const containers = document.querySelectorAll('[id^="edit"] > .showthread--anchored');

    containers.forEach((el) => {
      const authorEl = el.querySelector('.usernamedisplay__username');
      const author = authorEl?.textContent?.trim() || 'Unknown';

      const dateEl = el.querySelector('.threadpost-header__controllink--nolink .visible-dv');
      let date = dateEl?.textContent?.trim() || '';
      date = date.replace(/\s+/g, ' ').replace(/\s*\|.*$/, '').trim();

      const numEl = el.querySelector('.postnum') as HTMLElement | null;
      const postNumber = numEl?.dataset?.postnum || numEl?.textContent?.trim()?.replace('#', '') || '';

      const contentEl = el.querySelector('.threadpost-content__message');

      let content = '';
      if (contentEl) {
        const clone = contentEl.cloneNode(true) as HTMLElement;

        // Summarize quoted posts
        const quotes = clone.querySelectorAll('.quoteauthor, blockquote, [class*="quote"]');
        quotes.forEach((q) => {
          const quoteText = q.textContent?.trim().substring(0, 100) || '';
          const placeholder = document.createElement('span');
          placeholder.textContent = `[Quote: "${quoteText}..."]\n`;
          q.replaceWith(placeholder);
        });

        // Replace images with descriptive text
        const images = clone.querySelectorAll('img');
        images.forEach((img) => {
          const isEmoji = img.classList.contains('emoji');
          const placeholder = document.createElement('span');
          placeholder.textContent = isEmoji ? (img.alt || '') : `[Image: ${img.alt || 'attachment'}]`;
          img.replaceWith(placeholder);
        });

        // Convert <br> to newlines
        const brs = clone.querySelectorAll('br');
        brs.forEach((br) => {
          const newline = document.createElement('span');
          newline.textContent = '\n';
          br.replaceWith(newline);
        });

        content = clone.textContent?.trim() || '';
        content = content.replace(/\n{3,}/g, '\n\n');
      }

      if (content) {
        posts.push({ author, date, postNumber, content });
      }
    });

    return posts;
  });
}

async function extractThreadTitle(page: Page): Promise<string> {
  return page.evaluate(() => {
    const titleEl = document.querySelector('.showthread__title');
    return titleEl?.textContent?.trim() || 'ForexFactory Thread';
  });
}

async function navigateToNextPage(page: Page, currentPageNum: number): Promise<boolean> {
  const nextPageNum = currentPageNum + 1;
  const selector = `.visible-dv .scrollNav__navlist a[data-page="${nextPageNum}"]`;

  const exists = await page.$(selector);
  if (!exists) return false;

  try {
    await Promise.all([
      page.waitForNavigation({ waitUntil: 'networkidle2', timeout: 60000 }),
      page.click(selector),
    ]);
    await waitForContent(page);
    return true;
  } catch {
    // Cloudflare may re-challenge — give it extra time in headful mode
    try {
      await waitForContent(page);
      return true;
    } catch {
      return false;
    }
  }
}

function postsToMarkdown(title: string, url: string, posts: Post[]): string {
  const lines: string[] = [];
  lines.push(`# ${title}`);
  lines.push(`Source: ${url}`);
  lines.push(`Scraped: ${new Date().toISOString()}`);
  lines.push(`Total posts: ${posts.length}`);
  lines.push('');
  lines.push('---');
  lines.push('');

  for (const post of posts) {
    const header = [`**${post.author}**`];
    if (post.postNumber) header.push(`#${post.postNumber}`);
    if (post.date) header.push(`— ${post.date}`);
    lines.push(header.join(' '));
    lines.push('');
    lines.push(post.content);
    lines.push('');
    lines.push('---');
    lines.push('');
  }

  return lines.join('\n');
}

async function main() {
  const { url, maxPages, output, headless } = parseArgs();

  console.log(`Scraping ForexFactory thread: ${url}`);
  console.log(`Max pages: ${maxPages}`);
  console.log(`Mode: ${headless ? 'headless' : 'headful (visible browser)'}`);

  const browser = await puppeteer.launch({
    headless,
    args: ['--no-sandbox', '--disable-setuid-sandbox', '--window-size=1920,1080'],
    defaultViewport: { width: 1920, height: 1080 },
  });

  try {
    const page = await browser.newPage();

    await page.setUserAgent(
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36'
    );

    const allPosts: Post[] = [];
    let pageNum = 1;

    // Load first page
    console.log(`\nPage ${pageNum}: loading...`);
    await page.goto(url, { waitUntil: 'networkidle2', timeout: 60000 });
    await waitForContent(page);

    const title = await extractThreadTitle(page);
    console.log(`Thread: "${title}"`);

    while (pageNum <= maxPages) {
      const posts = await extractPosts(page);
      console.log(`  Found ${posts.length} posts`);

      if (posts.length === 0) {
        if (pageNum === 1) {
          const bodyText = await page.evaluate(() => document.body.innerText.substring(0, 500));
          console.error('  No posts found. Page preview:', bodyText);
          const html = await page.content();
          writeFileSync(join(process.cwd(), 'data', 'ff-debug.html'), html);
          console.error('  Saved debug HTML to data/ff-debug.html');
        }
        break;
      }

      allPosts.push(...posts);

      if (pageNum >= maxPages) break;

      console.log(`\nPage ${pageNum + 1}: navigating...`);
      await new Promise((r) => setTimeout(r, 3000));
      const navigated = await navigateToNextPage(page, pageNum);
      if (!navigated) {
        console.log('  No more pages or navigation failed.');
        break;
      }
      pageNum++;
    }

    if (allPosts.length === 0) {
      console.error('\nNo posts were scraped.');
      process.exit(1);
    }

    const markdown = postsToMarkdown(title, url, allPosts);

    const outputPath = output
      ? output
      : join(process.cwd(), 'data', `ff-thread-${Date.now()}.md`);

    writeFileSync(outputPath, markdown);
    console.log(`\nDone! ${allPosts.length} posts saved to: ${outputPath}`);
  } finally {
    await browser.close();
  }
}

main().catch((err) => {
  console.error('Fatal error:', err.message);
  process.exit(1);
});
