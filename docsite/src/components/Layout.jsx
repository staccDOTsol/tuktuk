import { useCallback, useEffect, useState } from 'react'
import Link from 'next/link'
import { useRouter } from 'next/router'
import clsx from 'clsx'
import { navigation } from '@/data/navigation'
import Image from 'next/image'
import { MobileNavigation } from '@/components/MobileNavigation'
import { Navigation } from '@/components/Navigation'
import { Prose } from '@/components/Prose'
import { Search } from '@/components/Search'
import Footer from '@/components/Footer'


function GitHubIcon(props) {
  return (
    <svg aria-hidden="true" viewBox="0 0 16 16" {...props}>
      <path d="M8 0C3.58 0 0 3.58 0 8C0 11.54 2.29 14.53 5.47 15.59C5.87 15.66 6.02 15.42 6.02 15.21C6.02 15.02 6.01 14.39 6.01 13.72C4 14.09 3.48 13.23 3.32 12.78C3.23 12.55 2.84 11.84 2.5 11.65C2.22 11.5 1.82 11.13 2.49 11.12C3.12 11.11 3.57 11.7 3.72 11.94C4.44 13.15 5.59 12.81 6.05 12.6C6.12 12.08 6.33 11.73 6.56 11.53C4.78 11.33 2.92 10.64 2.92 7.58C2.92 6.71 3.23 5.99 3.74 5.43C3.66 5.23 3.38 4.41 3.82 3.31C3.82 3.31 4.49 3.1 6.02 4.13C6.66 3.95 7.34 3.86 8.02 3.86C8.7 3.86 9.38 3.95 10.02 4.13C11.55 3.09 12.22 3.31 12.22 3.31C12.66 4.41 12.38 5.23 12.3 5.43C12.81 5.99 13.12 6.7 13.12 7.58C13.12 10.65 11.25 11.33 9.47 11.53C9.76 11.78 10.01 12.26 10.01 13.01C10.01 14.08 10 14.94 10 15.21C10 15.42 10.15 15.67 10.55 15.59C13.71 14.53 16 11.53 16 8C16 3.58 12.42 0 8 0Z" />
    </svg>
  )
}

function Header({ navigation }) {
  let [isScrolled, setIsScrolled] = useState(false)
  let router = useRouter()
  let isHomePage = router.pathname === '/'

  useEffect(() => {
    function onScroll() {
      setIsScrolled(window.scrollY > 0)
    }
    onScroll()
    window.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      window.removeEventListener('scroll', onScroll)
    }
  }, [])

  return (
    <header
      className={clsx(
        'sticky top-0 h-14 z-50 flex flex-wrap items-center justify-between bg-white/[var(--bg-opacity-light)] px-4 shadow-md shadow-zinc-900/5 backdrop-blur-sm transition duration-500 dark:shadow-none sm:px-6 lg:px-8',
        isScrolled && isHomePage
          ? 'dark:bg-zinc-800/[var(--bg-opacity-light)] dark:backdrop-blur-md'
          : 'dark:bg-transparent',
        !isHomePage &&
        'dark:bg-zinc-950/[var(--bg-opacity-light)] dark:backdrop-blur-md'
      )}
    >
      <div className="mr-6 flex lg:hidden">
        <MobileNavigation navigation={navigation} />
      </div>
      <div className="relative flex flex-grow basis-0 items-center">
        <Link href="/" aria-label="Home page" className='flex gap-3'>
          <div className='flex items-center gap-2'>
            <Image src="/tuktuk-logo.png" alt="TukTuk Logo" width={32} height={32} />
            <span className='text-lg font-bold'>TukTuk Docs</span>
          </div>
        </Link>
      </div>
      <div className="relative flex basis-0 items-center justify-end gap-4 md:flex-grow">
        <Search />
        <Link href="https://github.com/helium/tuktuk" className="group" aria-label="GitHub">
          <GitHubIcon className="h-6 w-6 fill-zinc-400 group-hover:fill-zinc-500 dark:group-hover:fill-zinc-300" />
        </Link>
      </div>
    </header>
  )
}

function useTableOfContents(tableOfContents) {
  let [currentSection, setCurrentSection] = useState(tableOfContents[0]?.id)

  let getHeadings = useCallback((tableOfContents) => {
    return tableOfContents
      .flatMap((node) => [node.id, ...node.children.map((child) => child.id)])
      .map((id) => {
        let el = document.getElementById(id)
        if (!el) return

        let style = window.getComputedStyle(el)
        let scrollMt = parseFloat(style.scrollMarginTop)

        let top = window.scrollY + el.getBoundingClientRect().top - scrollMt
        return { id, top }
      })
  }, [])

  useEffect(() => {
    if (tableOfContents.length === 0) return
    let headings = getHeadings(tableOfContents)
    function onScroll() {
      let top = window.scrollY
      let current = headings[0].id
      for (let heading of headings) {
        if (top >= heading.top) {
          current = heading.id
        } else {
          break
        }
      }
      setCurrentSection(current)
    }
    window.addEventListener('scroll', onScroll, { passive: true })
    onScroll()
    return () => {
      window.removeEventListener('scroll', onScroll)
    }
  }, [getHeadings, tableOfContents])

  return currentSection
}

export function Layout({ children, title, tableOfContents }) {
  let router = useRouter()
  let isHomePage = router.pathname === '/'
  let allLinks = navigation.flatMap((section) => section.links)
  let linkIndex = allLinks.findIndex((link) => link.href === router.pathname)
  let previousPage = allLinks[linkIndex - 1]
  let nextPage = allLinks[linkIndex + 1]
  let section = navigation.find((section) =>
    section.links.find((link) => link.href === router.pathname)
  )
  let currentSection = useTableOfContents(tableOfContents)

  function isActive(section) {
    if (section.id === currentSection) {
      return true
    }
    if (!section.children) {
      return false
    }
    return section.children.findIndex(isActive) > -1
  }

  return (
    <>
      <Header navigation={navigation} />

      <div className="relative mx-auto flex justify-center sm:px-2 lg:px-0">
        <div className="hidden lg:relative lg:block lg:flex-none">
          <div className="absolute inset-y-0 right-0 w-[50vw] bg-zinc-50 dark:hidden" />
          <div className="absolute bottom-0 right-0 top-16 hidden h-12 w-px bg-gradient-to-t from-zinc-800 dark:block" />
          <div className="absolute bottom-0 right-0 top-28 hidden w-px bg-zinc-800 dark:block" />
          <div className="sticky top-[3.5rem] -ml-0.5 h-[calc(100vh-3.5rem)] w-64 overflow-y-auto overflow-x-hidden bg-white py-10 pl-8 pr-8 xl:w-64 xl:pl-10 border-r border-gray-200">
            <Navigation navigation={navigation} />
          </div>
        </div>
        <div className="min-w-0 max-w-2xl flex-auto px-4 py-10 lg:max-w-none lg:px-8 xl:px-16">
          <div className="lg:max-w-5xl">
            <article>
              {(title || section) && (
                <header className="mb-2 space-y-1">
                  {section && (
                    <p className="font-display text-sm font-medium text-purple-700">
                      {section.title}
                    </p>
                  )}
                  {title && (
                    <h1 className="font-display text-3xl tracking-tight text-zinc-900 dark:text-white">
                      {title}
                    </h1>
                  )}
                </header>
              )}
              <Prose>{children}</Prose>
            </article>
          </div>
          <dl className="mt-12 flex pt-6">
            {previousPage && (
              <div>
                <dt>
                  <Link
                    href={previousPage.href}
                    className="inline-flex justify-center gap-0.5 overflow-hidden rounded-full bg-zinc-100 px-3 py-1 text-sm font-medium text-zinc-900 transition hover:bg-zinc-200 dark:bg-zinc-800/40 dark:text-zinc-400 dark:ring-1 dark:ring-inset dark:ring-zinc-800 dark:hover:bg-zinc-800 dark:hover:text-zinc-300"
                  >
                    <span aria-hidden="true">&larr;</span> Previous
                  </Link>
                </dt>
                <dd className="mt-1">
                  <Link
                    tabIndex="-1"
                    aria-hidden="true"
                    href={previousPage.href}
                    className="text-base font-semibold text-zinc-900 transition hover:text-zinc-600 dark:text-white dark:hover:text-zinc-300"
                  >
                    {previousPage.title}
                  </Link>
                </dd>
              </div>
            )}
            {nextPage && (
              <div className="ml-auto text-right">
                <dt>
                  <Link
                    href={nextPage.href}
                    className="inline-flex justify-center gap-0.5 overflow-hidden rounded-full bg-zinc-100 px-3 py-1 text-sm font-medium text-zinc-900 transition hover:bg-zinc-200 dark:bg-zinc-800/40 dark:text-zinc-400 dark:ring-1 dark:ring-inset dark:ring-zinc-800 dark:hover:bg-zinc-800 dark:hover:text-zinc-300"
                  >
                    Next <span aria-hidden="true">&rarr;</span>
                  </Link>
                </dt>
                <dd className="mt-1">
                  <Link
                    tabIndex="-1"
                    aria-hidden="true"
                    href={nextPage.href}
                    className="text-base font-semibold text-zinc-900 transition hover:text-zinc-600 dark:text-white dark:hover:text-zinc-300"
                  >
                    {nextPage.title}
                  </Link>
                </dd>
              </div>
            )}
          </dl>
          <Footer />
        </div>
        <div className="hidden xl:sticky xl:top-[3.5rem] xl:block xl:h-[calc(100vh-3.5rem)] xl:flex-none xl:overflow-y-auto xl:py-10 xl:pr-6 border-l border-gray-200 pl-6">
          <nav aria-labelledby="on-this-page-title" className="w-56">
            {tableOfContents.length > 0 && (
              <>
                <h2
                  id="on-this-page-title"
                  className="font-display text-sm font-medium text-zinc-900 dark:text-white"
                >
                  On this page
                </h2>
                <ol role="list" className="mt-4 space-y-3 text-sm">
                  {tableOfContents.map((section) => (
                    <li key={section.id}>
                      <h3>
                        <Link
                          href={`#${section.id}`}
                          className={clsx(
                            isActive(section)
                              ? 'text-purple-700'
                              : 'font-normal text-zinc-500 hover:text-zinc-700 dark:text-zinc-400 dark:hover:text-zinc-300'
                          )}
                        >
                          {section.title}
                        </Link>
                      </h3>
                      {section.children.length > 0 && (
                        <ol
                          role="list"
                          className="mt-2 space-y-3 pl-5 text-zinc-500 dark:text-zinc-400"
                        >
                          {section.children.map((subSection) => (
                            <li key={subSection.id}>
                              <Link
                                href={`#${subSection.id}`}
                                className={
                                  isActive(subSection)
                                    ? 'text-purple-700'
                                    : 'hover:text-zinc-600 dark:hover:text-zinc-300'
                                }
                              >
                                {subSection.title}
                              </Link>
                            </li>
                          ))}
                        </ol>
                      )}
                    </li>
                  ))}
                </ol>
              </>
            )}
          </nav>
        </div>
      </div>
    </>
  )
}
